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
    pub has_sub_stream: bool,
    pub enabled: bool,
    pub record_enabled: bool,
    pub storage_mode: String,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Serialize)]
pub struct CameraView {
    pub id: Uuid,
    pub name: String,
    pub location: String,
    pub has_sub_stream: bool,
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

#[cfg(test)]
mod camera_view_tests {
    use super::*;

    #[test]
    fn stored_camera_metadata_must_be_decoded_without_silent_defaults() {
        let now = Utc::now();
        let mut record = CameraRecord {
            id: Uuid::new_v4(),
            name: "front".to_owned(),
            location: String::new(),
            source_kind: "client".to_owned(),
            client_id: Some(Uuid::new_v4()),
            adapter_kind: "onvif".to_owned(),
            manufacturer: Some("Acme".to_owned()),
            model: None,
            firmware_version: None,
            serial_number: None,
            capabilities_json: r#"{"video":"supported","main_stream":"supported","sub_stream":"unsupported","local_recording":"supported","server_recording":"supported","ptz":"unsupported","events":"unsupported","audio_input":"unsupported","audio_output":"unsupported"}"#.to_owned(),
            streams_json: r#"[{"profile":"main","video_codec":"h264","audio_codec":null,"width":1920,"height":1080,"frame_rate":25.0}]"#.to_owned(),
            health_message: None,
            device_status: "online".to_owned(),
            has_sub_stream: false,
            enabled: true,
            record_enabled: true,
            storage_mode: "server".to_owned(),
            status: "online".to_owned(),
            last_seen_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        let view = CameraView::from_record(&record).unwrap();
        assert!(view.capabilities.video.is_supported());
        assert_eq!(view.streams[0].video_codec.as_deref(), Some("h264"));

        record.capabilities_json = "{}".to_owned();
        assert!(CameraView::from_record(&record).is_err());
        record.capabilities_json = r#"{"video":"supported","main_stream":"supported","sub_stream":"unsupported","local_recording":"supported","server_recording":"supported","ptz":"unsupported","events":"unsupported","audio_input":"unsupported","audio_output":"unsupported"}"#.to_owned();
        record.streams_json = "[{}]".to_owned();
        assert!(CameraView::from_record(&record).is_err());
    }
}

impl CameraView {
    pub fn from_record(value: &CameraRecord) -> Result<Self, serde_json::Error> {
        Ok(Self {
            id: value.id,
            name: value.name.clone(),
            location: value.location.clone(),
            has_sub_stream: value.has_sub_stream,
            source_kind: value.source_kind.clone(),
            client_id: value.client_id,
            adapter_kind: value.adapter_kind.clone(),
            manufacturer: value.manufacturer.clone(),
            model: value.model.clone(),
            firmware_version: value.firmware_version.clone(),
            serial_number: value.serial_number.clone(),
            capabilities: serde_json::from_str(&value.capabilities_json)?,
            streams: serde_json::from_str(&value.streams_json)?,
            health_message: value.health_message.clone(),
            device_status: value.device_status.clone(),
            storage_mode: value.storage_mode.clone(),
            enabled: value.enabled,
            record_enabled: value.record_enabled,
            status: value.status.clone(),
            last_seen_at: value.last_seen_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCapabilities {
    pub video: CapabilitySupport,
    pub main_stream: CapabilitySupport,
    pub sub_stream: CapabilitySupport,
    pub local_recording: CapabilitySupport,
    pub server_recording: CapabilitySupport,
    pub ptz: CapabilitySupport,
    pub events: CapabilitySupport,
    pub audio_input: CapabilitySupport,
    pub audio_output: CapabilitySupport,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}
impl CapabilitySupport {
    pub fn is_supported(self) -> bool {
        self == Self::Supported
    }
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

fn default_true() -> bool {
    true
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

pub const CLIENT_PAIRING_PROTOCOL: &str = env!("XCOS_EDGE_PROTOCOL");

#[derive(Clone, sqlx::FromRow)]
pub struct XcosClientRecord {
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
pub struct XcosClientView {
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
pub struct UpdateXcosClientRequest {
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateXcosClientRequest {
    pub name: Option<String>,
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
    pub outcome: CommandOutcome,
    pub error_code: Option<CommandErrorCode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutcome {
    Succeeded,
    Failed,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandErrorCode {
    InvalidCommand,
    UnsupportedCapability,
    ExpiredBeforeExecution,
    DeviceUnavailable,
    DeviceRejected,
    OutcomeUnknown,
}
impl CommandErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidCommand => "invalid_command",
            Self::UnsupportedCapability => "unsupported_capability",
            Self::ExpiredBeforeExecution => "expired_before_execution",
            Self::DeviceUnavailable => "device_unavailable",
            Self::DeviceRejected => "device_rejected",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientSnapshotRequest {
    pub protocol: String,
    pub command_capacity: usize,
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
    pub expires_at: DateTime<Utc>,
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
        rejects_unknown::<StreamTicketQuery>(json!({ "unknown": true }));
        rejects_unknown::<PtzRequest>(json!({ "action": "stop", "unknown": true }));
        rejects_unknown::<EventQuery>(json!({ "unknown": true }));
        rejects_unknown::<UpdateAuthorizationCodeRequest>(
            json!({ "authorization_code": "secret", "unknown": true }),
        );
        rejects_unknown::<UpdateXcosClientRequest>(json!({ "name": "renamed", "unknown": true }));
        rejects_unknown::<PairClientRequest>(json!({
            "protocol": CLIENT_PAIRING_PROTOCOL, "product": "xcos",
            "installation_id": Uuid::new_v4(), "name": "edge", "client_version": "0.1.0",
            "authorization_code": "secret", "unknown": true
        }));
        rejects_unknown::<ClientSnapshotRequest>(json!({
            "protocol": CLIENT_PAIRING_PROTOCOL, "command_capacity": 100, "cameras": [], "unknown": true
        }));
    }

    #[test]
    fn required_public_request_fields_cannot_be_omitted() {
        let create = serde_json::from_value::<CreateXcosClientRequest>(json!({})).unwrap();
        assert!(create.name.is_none());
        assert!(serde_json::from_value::<PtzRequest>(json!({})).is_err());
        assert!(serde_json::from_value::<ClientSnapshotRequest>(json!({
            "protocol": CLIENT_PAIRING_PROTOCOL, "command_capacity": 100,
            "cameras": []
        }))
        .is_err());
    }

    #[test]
    fn current_edge_snapshot_has_one_strict_vendor_neutral_shape() {
        let request = serde_json::from_value::<ClientSnapshotRequest>(json!({
            "protocol": CLIENT_PAIRING_PROTOCOL, "command_capacity": 100,
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
                    "video": "supported",
                    "main_stream": "supported",
                    "sub_stream": "unsupported",
                    "local_recording": "supported",
                    "server_recording": "supported",
                    "ptz": "supported",
                    "events": "unsupported",
                    "audio_input": "unsupported",
                    "audio_output": "unsupported"
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
        assert!(request.cameras[0].capabilities.ptz.is_supported());
    }
}
