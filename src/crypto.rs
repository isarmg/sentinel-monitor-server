//! Sentinel-specific credential domain and object binding over Foundation secrets.

use crate::error::{AppError, Result};
use sarmg_secret::{SecretBytes, SecretKey};
use sarmg_secret_envelope::EnvelopeDomain;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const PRODUCT: &str = "sentinel-monitor";
const APPLICATION_VERSION: &str = "0.2.2";
pub(crate) const CREDENTIAL_ENVELOPE_REVISION: u32 = 1;
const MAX_AUTHORIZATION_ENVELOPE_BYTES: usize = 1024;

struct ClientAuthorizationEnvelope;

impl EnvelopeDomain for ClientAuthorizationEnvelope {
    const DOMAIN: &'static [u8] = b"sentinel-monitor/client-authorization";
    const REVISION: u16 = CREDENTIAL_ENVELOPE_REVISION as u16;
}

#[derive(Clone)]
pub struct SecretBox {
    master: Arc<SecretKey<32>>,
}

impl SecretBox {
    pub fn new(master_key: &[u8; 32]) -> Self {
        Self {
            master: Arc::new(SecretKey::new(*master_key)),
        }
    }

    pub fn encrypt_client_authorization(
        &self,
        client_id: &str,
        plaintext: &str,
    ) -> Result<Vec<u8>> {
        sarmg_secret_envelope::seal::<ClientAuthorizationEnvelope>(
            &self.master,
            &authorization_binding(client_id),
            &SecretBytes::new(plaintext.as_bytes().to_vec()),
        )
        .map_err(|_| AppError::Internal("client authorization encryption failed".into()))
    }

    pub fn decrypt_client_authorization(&self, client_id: &str, encoded: &[u8]) -> Result<String> {
        if !(64..=MAX_AUTHORIZATION_ENVELOPE_BYTES).contains(&encoded.len()) {
            return Err(malformed_envelope());
        }
        let plaintext = sarmg_secret_envelope::open::<ClientAuthorizationEnvelope>(
            &self.master,
            &authorization_binding(client_id),
            encoded,
        )
        .map_err(|_| malformed_envelope())?;
        String::from_utf8(plaintext.expose().to_vec()).map_err(|_| malformed_envelope())
    }
}

fn authorization_binding(client_id: &str) -> Vec<u8> {
    let value = client_id.as_bytes();
    let mut binding = Vec::with_capacity(value.len() + 8);
    binding.extend_from_slice(&(value.len() as u64).to_be_bytes());
    binding.extend_from_slice(value);
    binding
}

pub(crate) fn credential_contract_sha256() -> String {
    let contract = format!(
        "format=sarmg-secret-envelope\nproduct={PRODUCT}\napplication_version={APPLICATION_VERSION}\nenvelope_revision={CREDENTIAL_ENVELOPE_REVISION}\ndomain={}\nbinding=client_instance_id\nfield=authorization_code_enc\nmax_envelope_bytes={MAX_AUTHORIZATION_ENVELOPE_BYTES}\nauthorization_code=current:32-lowercase-alphanumeric;legacy-pairing:64-lowercase-hex\n",
        String::from_utf8_lossy(ClientAuthorizationEnvelope::DOMAIN),
    );
    format!("{:x}", Sha256::digest(contract.as_bytes()))
}

fn malformed_envelope() -> AppError {
    AppError::Internal("credential envelope is not exactly current or authenticated".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_under_test() -> SecretBox {
        SecretBox::new(&[0x42; 32])
    }

    #[test]
    fn credential_identity_and_schema_are_a_single_current_contract() {
        assert_eq!(APPLICATION_VERSION, "0.2.2");
        let schema = include_str!("../schema/generated/current_schema.sql");
        assert!(schema.contains("authorization_code_enc BLOB"));
        let camera_table = schema
            .split_once("CREATE TABLE cameras (")
            .and_then(|(_, remainder)| remainder.split_once("\n);"))
            .map(|(table, _)| table)
            .expect("current schema must contain the cameras table");
        for removed in [
            "main_stream_url_enc",
            "sub_stream_url_enc",
            "onvif_url",
            "username_enc",
            "password_enc",
        ] {
            assert!(!camera_table.contains(removed));
        }
    }

    #[test]
    fn current_envelope_is_randomized_and_context_bound() {
        let secrets = box_under_test();
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        let code = "a".repeat(32);
        let encoded = secrets.encrypt_client_authorization(&first, &code).unwrap();
        let another = secrets.encrypt_client_authorization(&first, &code).unwrap();
        assert_ne!(encoded, another);
        assert_eq!(&encoded[..4], b"SGEV");
        assert_eq!(
            secrets
                .decrypt_client_authorization(&first, &encoded)
                .unwrap(),
            code
        );
        assert!(secrets
            .decrypt_client_authorization(&second, &encoded)
            .is_err());
        assert!(SecretBox::new(&[0x99; 32])
            .decrypt_client_authorization(&first, &encoded)
            .is_err());
    }

    #[test]
    fn malformed_and_tampered_values_fail_without_secret_disclosure() {
        let secrets = box_under_test();
        let client_id = uuid::Uuid::new_v4().to_string();
        let secret = "b".repeat(32);
        let mut tampered = secrets
            .encrypt_client_authorization(&client_id, &secret)
            .unwrap();
        *tampered.last_mut().unwrap() ^= 1;

        for invalid in [b"not-an-envelope".to_vec(), tampered] {
            let error = secrets
                .decrypt_client_authorization(&client_id, &invalid)
                .unwrap_err()
                .to_string();
            assert!(!error.contains(&secret));
            assert!(!error.contains(&client_id));
        }
    }

    #[test]
    fn client_authorization_is_encrypted_and_bound_to_one_instance() {
        let secrets = box_under_test();
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        let code = "a".repeat(32);
        let encrypted = secrets.encrypt_client_authorization(&first, &code).unwrap();
        assert!(!encrypted
            .windows(code.len())
            .any(|value| value == code.as_bytes()));
        assert_eq!(
            secrets
                .decrypt_client_authorization(&first, &encrypted)
                .unwrap(),
            code
        );
        assert!(secrets
            .decrypt_client_authorization(&second, &encrypted)
            .is_err());
    }
}
