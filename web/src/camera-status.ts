export type CameraObservation = {
  observation_status: "fresh" | "stale" | "unknown";
  last_observed_at: string | null;
  observation_expires_at: string | null;
};

type CameraStatus = CameraObservation & {
  enabled: boolean;
  status: string;
  device_status: string;
};

const isTimestamp = (value: unknown): value is string =>
  typeof value === "string" && Number.isFinite(Date.parse(value));

export function isCameraObservation(value: Record<string, unknown>): boolean {
  const observed = value.last_observed_at;
  const expires = value.observation_expires_at;
  if (observed === null) return value.observation_status === "unknown" && expires === null;
  if (!isTimestamp(observed)) return false;
  if (expires !== null && (!isTimestamp(expires) || Date.parse(expires) <= Date.parse(observed))) return false;
  return value.observation_status === "stale" || (value.observation_status === "fresh" && expires !== null);
}

export function effectiveStatus(camera: CameraStatus, now = Date.now()): string {
  if (!camera.enabled) return "disabled";
  if (camera.observation_status !== "fresh") return camera.observation_status;
  if (camera.observation_expires_at === null || Date.parse(camera.observation_expires_at) <= now) return "stale";
  return camera.device_status === "online" ? camera.status : camera.device_status;
}
