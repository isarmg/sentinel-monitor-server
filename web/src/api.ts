import { createAdministratorApiClient, type JsonGuard } from "@sarmg/admin-web";
import { ADMIN_AUTH_PATHS } from "@sarmg/contracts";

import protocolContract from "./protocol-contract.json";

const LOGIN_SUFFIX = "/auth/login";
if (!ADMIN_AUTH_PATHS.login.endsWith(LOGIN_SUFFIX)) {
  throw new Error("Foundation 管理登录路径不符合当前合同");
}
const foundationApiPrefix = ADMIN_AUTH_PATHS.login.slice(0, -LOGIN_SUFFIX.length);
if (protocolContract.api_prefix !== foundationApiPrefix) {
  throw new Error("Sentinel 产品协议必须与 Foundation 管理 API 使用同一前缀");
}

export const administratorApi = createAdministratorApiClient();

export type Camera = {
  id: string;
  name: string;
  location: string;
  has_sub_stream: boolean;
  source_kind: "client";
  client_id: string | null;
  adapter_kind: string;
  manufacturer: string | null;
  model: string | null;
  firmware_version: string | null;
  serial_number: string | null;
  capabilities: CameraCapabilities;
  streams: CameraStream[];
  health_message: string | null;
  device_status: string;
  storage_mode: "client" | "server";
  enabled: boolean;
  record_enabled: boolean;
  status: string;
  last_seen_at: string | null;
  created_at: string;
  updated_at: string;
};

export type CameraCapabilities = {
  video: boolean;
  main_stream: boolean;
  sub_stream: boolean;
  local_recording: boolean;
  server_recording: boolean;
  ptz: boolean;
  events: boolean;
  audio_input: boolean;
  audio_output: boolean;
};

export type CameraStream = {
  profile: "main" | "sub";
  video_codec: string | null;
  audio_codec: string | null;
  width: number | null;
  height: number | null;
  frame_rate: number | null;
};

export type MediaOperation = {
  id: string;
  camera_id: string;
  generation: number;
  kind: string;
  state: string;
  reason: string;
  requested_by: string | null;
  attempt: number;
  max_attempts: number;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  retry_at: string | null;
  error_code: string | null;
  error_message: string | null;
};

export type StreamTicket = {
  profile: string;
  whep_url: string;
  hls_url: string;
  token: string;
  expires_at: string;
};

export type RecordingSpan = { start: string; duration: number };

export type MonitorEvent = {
  id: string;
  camera_id: string | null;
  kind: string;
  severity: "info" | "warning" | "critical";
  message: string;
  acknowledged_at: string | null;
  created_at: string;
};

export type AuditRow = {
  id: string;
  user_id: string | null;
  action: string;
  entity_type: string;
  entity_id: string | null;
  details: Record<string, unknown>;
  created_at: string;
};

export type SystemStatus = {
  service: string;
  version: string;
  database: string;
  media_service: string;
  cameras: { recording_configured: number };
  server_time: string;
};

export type SentinelClient = {
  id: string;
  installation_id: string | null;
  name: string;
  client_version: string | null;
  authorization_code: string;
  status: "pending" | "online" | "offline" | "revoked";
  last_seen_at: string | null;
  created_at: string;
  updated_at: string;
};

export function apiPath(path: string): string {
  if (!path.startsWith("/") || path.startsWith("//")) {
    throw new TypeError("业务 API 路径必须是单斜杠开头的绝对应用路径");
  }
  return `${protocolContract.api_prefix}${path}`;
}

export function request<T>(
  path: string,
  guard: JsonGuard<T>,
  init?: RequestInit,
): Promise<T> {
  return administratorApi.request(apiPath(path), guard, init);
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const isString = (value: unknown): value is string => typeof value === "string";
const isBoolean = (value: unknown): value is boolean => typeof value === "boolean";
const isNumber = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);
const isNullableString = (value: unknown): value is string | null =>
  value === null || isString(value);
const isNullableNumber = (value: unknown): value is number | null =>
  value === null || isNumber(value);
const arrayOf = <T>(guard: JsonGuard<T>): JsonGuard<T[]> =>
  (value): value is T[] => Array.isArray(value) && value.every(guard);

const isCameraCapabilities: JsonGuard<CameraCapabilities> = (value): value is CameraCapabilities =>
  isRecord(value) &&
  ["video", "main_stream", "sub_stream", "local_recording", "server_recording", "ptz", "events", "audio_input", "audio_output"].every((key) => isBoolean(value[key]));

const isCameraStream: JsonGuard<CameraStream> = (value): value is CameraStream =>
  isRecord(value) &&
  ["main", "sub"].includes(value.profile as string) &&
  isNullableString(value.video_codec) &&
  isNullableString(value.audio_codec) &&
  isNullableNumber(value.width) &&
  isNullableNumber(value.height) &&
  isNullableNumber(value.frame_rate);

export const isUndefined = (value: unknown): value is undefined => value === undefined;

export const isCamera: JsonGuard<Camera> = (value): value is Camera =>
  isRecord(value) &&
  ["id", "name", "location", "status", "device_status", "adapter_kind", "created_at", "updated_at"].every((key) =>
    isString(value[key]),
  ) &&
  isNullableString(value.client_id) &&
  isNullableString(value.manufacturer) &&
  isNullableString(value.model) &&
  isNullableString(value.firmware_version) &&
  isNullableString(value.serial_number) &&
  isNullableString(value.health_message) &&
  value.source_kind === "client" &&
  ["client", "server"].includes(value.storage_mode as string) &&
  isNullableString(value.last_seen_at) &&
  isBoolean(value.has_sub_stream) &&
  isBoolean(value.enabled) &&
  isBoolean(value.record_enabled) &&
  isCameraCapabilities(value.capabilities) &&
  Array.isArray(value.streams) &&
  value.streams.every(isCameraStream);

export const isCameras = arrayOf(isCamera);

export const isOperation: JsonGuard<MediaOperation> = (
  value,
): value is MediaOperation =>
  isRecord(value) &&
  ["id", "camera_id", "kind", "state", "reason", "created_at"].every((key) =>
    isString(value[key]),
  ) &&
  ["generation", "attempt", "max_attempts"].every((key) => isNumber(value[key])) &&
  [
    "requested_by",
    "started_at",
    "finished_at",
    "retry_at",
    "error_code",
    "error_message",
  ].every((key) => isNullableString(value[key]));
export const isOperations = arrayOf(isOperation);

export const isStreamTicket: JsonGuard<StreamTicket> = (
  value,
): value is StreamTicket =>
  isRecord(value) &&
  ["profile", "whep_url", "hls_url", "token", "expires_at"].every((key) =>
    isString(value[key]),
  );

const isRecordingSpan: JsonGuard<RecordingSpan> = (
  value,
): value is RecordingSpan =>
  isRecord(value) && isString(value.start) && isNumber(value.duration);
export const isRecordingSpans = arrayOf(isRecordingSpan);

const isMonitorEvent: JsonGuard<MonitorEvent> = (
  value,
): value is MonitorEvent =>
  isRecord(value) &&
  ["id", "kind", "severity", "message", "created_at"].every((key) =>
    isString(value[key]),
  ) &&
  isNullableString(value.camera_id) &&
  isNullableString(value.acknowledged_at) &&
  ["info", "warning", "critical"].includes(value.severity as string);
export const isMonitorEvents = arrayOf(isMonitorEvent);

const isAuditRow: JsonGuard<AuditRow> = (value): value is AuditRow =>
  isRecord(value) &&
  ["id", "action", "entity_type", "created_at"].every((key) => isString(value[key])) &&
  isNullableString(value.user_id) && isNullableString(value.entity_id) && isRecord(value.details);
export const isAuditRows = arrayOf(isAuditRow);

export const isSystemStatus: JsonGuard<SystemStatus> = (
  value,
): value is SystemStatus =>
  isRecord(value) &&
  ["service", "version", "database", "media_service", "server_time"].every((key) =>
    isString(value[key]),
  ) &&
  isRecord(value.cameras) &&
  isNumber(value.cameras.recording_configured);


export const isSentinelClient: JsonGuard<SentinelClient> = (value): value is SentinelClient =>
  isRecord(value) &&
  ["id", "name", "authorization_code", "status", "created_at", "updated_at"].every((key) => isString(value[key])) &&
  /^[a-z0-9]{36}$/.test(value.authorization_code as string) &&
  isNullableString(value.installation_id) && isNullableString(value.client_version) &&
  isNullableString(value.last_seen_at) && ["pending", "online", "offline", "revoked"].includes(value.status as string);
export const isSentinelClients = arrayOf(isSentinelClient);
