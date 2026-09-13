use crate::{
    crypto::{CredentialField, SecretBox},
    error::Result,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, sqlx::FromRow)]
pub struct CameraRecord {
    pub id: Uuid,
    pub name: String,
    pub location: String,
    pub source_kind: String,
    pub client_id: Option<Uuid>,
    pub adapter_kind: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
    pub capabilities_json: String,
    pub streams_json: String,
    pub health_message: Option<String>,
    pub device_status: String,
    pub main_stream_url_enc: Option<Vec<u8>>,
    pub sub_stream_url_enc: Option<Vec<u8>>,
    pub has_sub_stream: bool,
    pub onvif_url: Option<String>,
    pub username_enc: Option<Vec<u8>>,
    pub password_enc: Option<Vec<u8>>,
    pub enabled: bool,
    pub record_enabled: bool,
    pub storage_mode: String,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct CameraCredentials {
    pub main_stream_url: Option<String>,
    pub sub_stream_url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl CameraRecord {
    pub fn decrypt_credentials(&self, secrets: &SecretBox) -> Result<CameraCredentials> {
        Ok(CameraCredentials {
            main_stream_url: self
                .main_stream_url_enc
                .as_deref()
                .map(|value| secrets.decrypt(self.id, CredentialField::MainStreamUrl, value))
                .transpose()?,
            sub_stream_url: self
                .sub_stream_url_enc
                .as_deref()
                .map(|value| secrets.decrypt(self.id, CredentialField::SubStreamUrl, value))
                .transpose()?,
            username: self
                .username_enc
                .as_deref()
                .map(|value| secrets.decrypt(self.id, CredentialField::Username, value))
                .transpose()?,
            password: self
                .password_enc
                .as_deref()
                .map(|value| secrets.decrypt(self.id, CredentialField::Password, value))
                .transpose()?,
        })
    }
}

#[derive(Clone, Serialize)]
pub struct CameraView {
    pub id: Uuid,
    pub name: String,
    pub location: String,
    pub has_sub_stream: bool,
    pub onvif_configured: bool,
    pub username: Option<String>,
    pub source_kind: String,
    pub client_id: Option<Uuid>,
    pub adapter_kind: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
    pub capabilities: DeviceCapabilities,
    pub streams: Vec<StreamDescriptor>,
    pub health_message: Option<String>,
    pub device_status: String,
    pub storage_mode: String,
    pub enabled: bool,
    pub record_enabled: bool,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CameraView {
    pub fn from_record(value: &CameraRecord, credentials: &CameraCredentials) -> Self {
        let capabilities = if value.source_kind == "direct" {
            DeviceCapabilities {
                video: true,
                main_stream: true,
                sub_stream: value.has_sub_stream,
                local_recording: false,
                server_recording: true,
                ptz: value.onvif_url.is_some(),
                ..DeviceCapabilities::default()
            }
        } else {
            serde_json::from_str(&value.capabilities_json).unwrap_or_default()
        };
        let streams = if value.source_kind == "direct" {
            let mut streams = vec![StreamDescriptor::unknown("main")];
            if value.has_sub_stream {
                streams.push(StreamDescriptor::unknown("sub"));
            }
            streams
        } else {
            serde_json::from_str(&value.streams_json).unwrap_or_default()
        };
        Self {
            id: value.id,
            name: value.name.clone(),
            location: value.location.clone(),
            has_sub_stream: value.has_sub_stream,
            onvif_configured: value.onvif_url.is_some(),
            username: credentials.username.clone(),
            source_kind: value.source_kind.clone(),
            client_id: value.client_id,
            adapter_kind: value.adapter_kind.clone(),
            manufacturer: value.manufacturer.clone(),
            model: value.model.clone(),
            firmware_version: value.firmware_version.clone(),
            serial_number: value.serial_number.clone(),
            capabilities,
            streams,
            health_message: value.health_message.clone(),
            device_status: if value.source_kind == "direct" {
                value.status.clone()
            } else {
                value.device_status.clone()
            },
            storage_mode: value.storage_mode.clone(),
            enabled: value.enabled,
            record_enabled: value.record_enabled,
            status: value.status.clone(),
            last_seen_at: value.last_seen_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCapabilities {
    pub video: bool,
    pub main_stream: bool,
    pub sub_stream: bool,
    pub local_recording: bool,
    pub server_recording: bool,
    pub ptz: bool,
    pub events: bool,
    pub audio_input: bool,
    pub audio_output: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StreamDescriptor {
    pub profile: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f64>,
}

impl StreamDescriptor {
    fn unknown(profile: &str) -> Self {
        Self {
            profile: profile.to_owned(),
            video_codec: None,
            audio_codec: None,
            width: None,
            height: None,
            frame_rate: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCameraRequest {
    pub name: String,
    #[serde(default)]
    pub location: String,
    pub main_stream_url: String,
    pub sub_stream_url: Option<String>,
    pub onvif_url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub record_enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCameraRequest {
    pub name: Option<String>,
    pub location: Option<String>,
    pub main_stream_url: Option<String>,
    pub sub_stream_url: Option<String>,
    #[serde(default)]
    pub clear_sub_stream: bool,
    pub onvif_url: Option<String>,
    #[serde(default)]
    pub clear_onvif: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub clear_password: bool,
    pub enabled: Option<bool>,
    pub record_enabled: Option<bool>,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
pub struct CameraMutationResponse {
    pub camera: CameraView,
    pub media_synced: bool,
    pub warning: Option<String>,
    pub operation_id: String,
    pub operation_state: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamTicketQuery {
    pub profile: Option<String>,
}

#[derive(Serialize)]
pub struct StreamTicket {
    pub profile: String,
    pub whep_url: String,
    pub hls_url: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PtzRequest {
    pub action: String,
    pub pan: Option<f64>,
    pub tilt: Option<f64>,
    pub zoom: Option<f64>,
}

pub const CLIENT_PAIRING_PROTOCOL: &str = "sentinel-edge-v2";

#[derive(Clone, sqlx::FromRow)]
pub struct SentinelClientRecord {
    pub id: Uuid,
    pub installation_id: Option<Uuid>,
    pub name: String,
    pub client_version: Option<String>,
    pub authorization_code_enc: Vec<u8>,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Serialize)]
pub struct SentinelClientView {
    pub id: Uuid,
    pub installation_id: Option<Uuid>,
    pub name: String,
    pub client_version: Option<String>,
    pub authorization_code: String,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAuthorizationCodeRequest {
    pub authorization_code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSentinelClientRequest {
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairClientRequest {
    pub protocol: String,
    pub product: String,
    pub installation_id: Uuid,
    pub name: String,
    pub client_version: String,
    pub authorization_code: String,
}

#[derive(Serialize)]
pub struct PairClientResponse {
    pub protocol: &'static str,
    pub client_id: Uuid,
    pub access_token: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientCameraSnapshot {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub location: String,
    pub adapter_kind: String,
    pub identity: DeviceIdentity,
    pub capabilities: DeviceCapabilities,
    pub streams: Vec<StreamDescriptor>,
    #[serde(default)]
    pub has_sub_stream: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub storage_mode: String,
    pub status: String,
    pub health_message: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentity {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCommandResult {
    pub id: Uuid,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientSnapshotRequest {
    pub protocol: String,
    pub cameras: Vec<ClientCameraSnapshot>,
    pub command_results: Vec<DeviceCommandResult>,
}

#[derive(Serialize)]
pub struct CameraPublishGrant {
    pub camera_id: Uuid,
    pub profile: String,
    pub publish_url: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DeviceCommand {
    pub id: Uuid,
    pub camera_id: Uuid,
    pub kind: String,
    pub payload: Value,
}

#[derive(Serialize)]
pub struct ClientSnapshotResponse {
    pub protocol: &'static str,
    pub accepted_at: DateTime<Utc>,
    pub publish: Vec<CameraPublishGrant>,
    pub commands: Vec<DeviceCommand>,
}

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct EventRecord {
    pub id: Uuid,
    pub camera_id: Option<Uuid>,
    pub kind: String,
    pub severity: String,
    pub message: String,
    pub details: Value,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub acknowledged_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventQuery {
    pub camera_id: Option<Uuid>,
    pub unacknowledged: Option<bool>,
    pub limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AuditRecord {
    pub id: Uuid,
    pub user_id: Option<String>,
    pub action: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub details: Value,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn rejects_unknown<T: DeserializeOwned>(value: Value) {
        assert!(serde_json::from_value::<T>(value).is_err());
    }

    #[test]
    fn every_public_request_dto_rejects_unknown_fields() {
        rejects_unknown::<CreateCameraRequest>(json!({
            "name": "Camera",
            "main_stream_url": "rtsp://camera.invalid/main",
            "unknown": true
        }));
        rejects_unknown::<UpdateCameraRequest>(json!({ "unknown": true }));
        rejects_unknown::<StreamTicketQuery>(json!({ "unknown": true }));
        rejects_unknown::<PtzRequest>(json!({ "action": "stop", "unknown": true }));
        rejects_unknown::<EventQuery>(json!({ "unknown": true }));
        rejects_unknown::<UpdateAuthorizationCodeRequest>(
            json!({ "authorization_code": "secret", "unknown": true }),
        );
        rejects_unknown::<PairClientRequest>(json!({
            "protocol": CLIENT_PAIRING_PROTOCOL, "product": "sentinel-monitor",
            "installation_id": Uuid::new_v4(), "name": "edge", "client_version": "0.1.0",
            "authorization_code": "secret", "unknown": true
        }));
        rejects_unknown::<ClientSnapshotRequest>(json!({
            "protocol": CLIENT_PAIRING_PROTOCOL, "cameras": [], "unknown": true
        }));
    }

    #[test]
    fn required_public_request_fields_cannot_be_omitted() {
        assert!(serde_json::from_value::<CreateCameraRequest>(json!({
            "main_stream_url": "rtsp://camera.invalid/main"
        }))
        .is_err());
        assert!(serde_json::from_value::<PtzRequest>(json!({})).is_err());
        assert!(serde_json::from_value::<ClientSnapshotRequest>(json!({
            "protocol": CLIENT_PAIRING_PROTOCOL,
            "cameras": []
        }))
        .is_err());
    }

    #[test]
    fn current_edge_snapshot_has_one_strict_vendor_neutral_shape() {
        let request = serde_json::from_value::<ClientSnapshotRequest>(json!({
            "protocol": CLIENT_PAIRING_PROTOCOL,
            "cameras": [{
                "id": Uuid::new_v4(),
                "name": "Warehouse PTZ",
                "location": "Warehouse",
                "adapter_kind": "onvif",
                "identity": {
                    "manufacturer": "Example",
                    "model": "IPC-1",
                    "firmware_version": "1.2.3",
                    "serial_number": "SERIAL"
                },
                "capabilities": {
                    "video": true,
                    "main_stream": true,
                    "sub_stream": false,
                    "local_recording": true,
                    "server_recording": true,
                    "ptz": true,
                    "events": false,
                    "audio_input": false,
                    "audio_output": false
                },
                "streams": [{
                    "profile": "main",
                    "video_codec": "H264",
                    "audio_codec": null,
                    "width": 1920,
                    "height": 1080,
                    "frame_rate": 25.0
                }],
                "has_sub_stream": false,
                "enabled": true,
                "storage_mode": "server",
                "status": "online",
                "health_message": null
            }],
            "command_results": []
        }))
        .unwrap();
        assert_eq!(request.cameras[0].adapter_kind, "onvif");
        assert!(request.cameras[0].capabilities.ptz);
    }
}
