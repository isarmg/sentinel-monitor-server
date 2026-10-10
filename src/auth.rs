use crate::{
    config::Config,
    error::{AppError, Result},
    protocol::CONTRACT,
    AppState,
};
use axum::{extract::FromRequestParts, http::request::Parts, response::Response};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hkdf::Hkdf;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MEDIA_JWT_KEY_SALT: &[u8] = b"xcos/1.0.0/media-jwt/signing-key";
const MEDIA_JWT_KEY_INFO: &[u8] = b"xcos-media-jwt-v1/HS256";

#[derive(Clone)]
pub struct CurrentUser {
    pub id: String,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> std::result::Result<Self, Self::Rejection> {
        let identity = xcss::admin_axum::authenticate_request(
            &state.administrator,
            &parts.headers,
            &parts.uri,
            &parts.method,
            "xcos",
            state.administrator_origin,
        )
        .await
        .map_err(|response| *response)?;
        let id = identity.administrator_id.to_string();
        Ok(Self { id })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaClaims {
    pub protocol: String,
    pub iss: String,
    pub aud: String,
    pub kind: String,
    pub sub: String,
    pub camera_id: Uuid,
    pub path: String,
    pub actions: Vec<String>,
    pub credential_binding: Option<String>,
    pub jti: Uuid,
    pub iat: u64,
    pub nbf: u64,
    pub exp: u64,
}

pub fn issue_media_token(
    user_id: &str,
    camera_id: Uuid,
    path: String,
    actions: Vec<String>,
    credential_binding: Option<String>,
    config: &Config,
) -> Result<(String, DateTime<Utc>)> {
    let now = Utc::now();
    let expires_at = now
        + ChronoDuration::from_std(config.media_token_ttl)
            .map_err(|_| AppError::Internal("invalid media token duration".into()))?;
    let claims = MediaClaims {
        protocol: CONTRACT.media_jwt_protocol.clone(),
        iss: CONTRACT.media_jwt_issuer.clone(),
        aud: CONTRACT.media_jwt_audience.clone(),
        kind: CONTRACT.media_jwt_kind.clone(),
        sub: user_id.to_owned(),
        camera_id,
        path,
        actions,
        credential_binding,
        jti: Uuid::new_v4(),
        iat: now.timestamp() as u64,
        nbf: now.timestamp() as u64,
        exp: expires_at.timestamp() as u64,
    };
    let signing_key = media_signing_key(&config.jwt_secret)?;
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&signing_key),
    )
    .map_err(|error| AppError::Internal(format!("media token failed: {error}")))?;
    Ok((token, expires_at))
}

pub fn decode_media_token(token: &str, config: &Config) -> Result<MediaClaims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.validate_nbf = true;
    validation.set_required_spec_claims(&["exp", "nbf", "aud", "iss", "sub"]);
    validation.set_audience(&[&CONTRACT.media_jwt_audience]);
    validation.set_issuer(&[&CONTRACT.media_jwt_issuer]);
    let signing_key = media_signing_key(&config.jwt_secret)?;
    let claims = decode::<MediaClaims>(token, &DecodingKey::from_secret(&signing_key), &validation)
        .map_err(|_| AppError::Unauthorized)?
        .claims;
    let now = u64::try_from(Utc::now().timestamp()).map_err(|_| AppError::Unauthorized)?;
    if claims.protocol != CONTRACT.media_jwt_protocol
        || claims.iss != CONTRACT.media_jwt_issuer
        || claims.aud != CONTRACT.media_jwt_audience
        || claims.kind != CONTRACT.media_jwt_kind
        || xcss::admin_core::Identifier::new(claims.sub.clone()).is_err()
        || claims.camera_id.is_nil()
        || claims.jti.is_nil()
        || claims.path.is_empty()
        || claims.path.len() > 512
        || claims.actions.is_empty()
        || claims.actions.len() > 4
        || claims
            .actions
            .iter()
            .any(|action| !matches!(action.as_str(), "read" | "playback" | "publish"))
        || if claims.actions.iter().any(|action| action == "publish") {
            claims.credential_binding.as_ref().is_none_or(|value| {
                value.len() != 64
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
        } else {
            claims.credential_binding.is_some()
        }
        || claims.nbf != claims.iat
        || claims.iat > now
        || claims.exp.checked_sub(claims.iat) != Some(config.media_token_ttl.as_secs())
    {
        return Err(AppError::Unauthorized);
    }
    Ok(claims)
}

pub fn publish_binding(token_hash: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"xcos/publish-binding/v1\0");
    digest.update(token_hash);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn media_signing_key(secret: &[u8]) -> Result<[u8; 32]> {
    let mut key = [0_u8; 32];
    Hkdf::<Sha256>::new(Some(MEDIA_JWT_KEY_SALT), secret)
        .expand(MEDIA_JWT_KEY_INFO, &mut key)
        .map_err(|_| AppError::Internal("media signing key derivation failed".into()))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::MediaClaims;
    use serde_json::{json, Value};
    use uuid::Uuid;

    fn current_claims() -> Value {
        json!({
            "protocol":"xcos-media-jwt-v1","iss":"xcos/1.0.0",
            "aud":"xcos-mediamtx/1.20.0","kind":"media","sub":Uuid::new_v4(),
            "camera_id":Uuid::new_v4(),"path":"camera/main","actions":["read"],"credential_binding":null,
            "jti":Uuid::new_v4(),"iat":1,"nbf":1,"exp":121
        })
    }

    #[test]
    fn current_media_key_and_hs256_signature_match_independent_known_answers() {
        // These fixture-only answers were calculated with Python's standard
        // HMAC-SHA256 and RFC 5869, independently of either Rust dependency.
        let key = super::media_signing_key(&[0x42; 32]).unwrap();
        assert_eq!(
            key.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            "c7313753865ea91c1d23d94906887392fa3c52baf7e7b6ef383e3f98065fadb5"
        );
        let message = b"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJmaXh0dXJlLXVzZXIiLCJwcm90b2NvbCI6Inhjb3MtbWVkaWEtand0LXYxIn0";
        let signature = jsonwebtoken::crypto::sign(
            message,
            &jsonwebtoken::EncodingKey::from_secret(&key),
            jsonwebtoken::Algorithm::HS256,
        )
        .unwrap();
        assert_eq!(signature, "QHt-81aQ8Ck76N0J6slIso7TgOtuK4_4aLDQam8OIio");
        assert!(jsonwebtoken::crypto::verify(
            &signature,
            message,
            &jsonwebtoken::DecodingKey::from_secret(&key),
            jsonwebtoken::Algorithm::HS256,
        )
        .unwrap());
        assert!(!jsonwebtoken::crypto::verify(
            &signature,
            b"tampered",
            &jsonwebtoken::DecodingKey::from_secret(&key),
            jsonwebtoken::Algorithm::HS256,
        )
        .unwrap());
    }

    #[test]
    fn media_claims_reject_unknown_and_missing_fields() {
        let mut unknown = current_claims();
        unknown
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), json!(true));
        assert!(serde_json::from_value::<MediaClaims>(unknown).is_err());
        let mut missing = current_claims();
        missing.as_object_mut().unwrap().remove("jti");
        assert!(serde_json::from_value::<MediaClaims>(missing).is_err());
    }

    #[test]
    fn media_claims_preserve_opaque_foundation_administrator_ids() {
        let identifier = xcss::admin_auth::random_token().unwrap();
        let mut value = current_claims();
        value["sub"] = json!(identifier);
        let claims: MediaClaims = serde_json::from_value(value).unwrap();
        assert_eq!(claims.sub, identifier);
        assert!(xcss::admin_core::Identifier::new(claims.sub).is_ok());
    }
}
