import { t } from "@sarmg/admin-ui/i18n";
const labels: Record<string, readonly [string, string]> = {
  "camera.online": ["摄像头已恢复在线", "Camera is back online"], "camera.offline": ["摄像头已离线", "Camera is offline"],
  "camera.status": ["摄像头状态变更", "Camera status changed"], "camera.updated": ["更新摄像头", "Update camera"],
  "camera.ptz.queued": ["云台命令已排队", "PTZ command queued"],
  "client.create": ["创建实例", "Create instance"], "client.pair": ["配对实例", "Pair instance"],
  "client.authorization.rotate": ["更换授权码", "Rotate authorization code"],
  "client.revoke": ["撤销实例", "Revoke instance"], "client.delete": ["删除实例", "Delete instance"],
  "event.acknowledge": ["确认事件", "Acknowledge event"], "event.ack": ["确认事件", "Acknowledge event"],
  "media.operation.resolve": ["核对媒体操作", "Reconcile media operation"],
  pending: ["待配对", "Awaiting pairing"], online: ["在线", "Online"], offline: ["离线", "Offline"], revoked: ["已撤销", "Revoked"],
  camera: ["摄像头", "Camera"], event: ["事件", "Event"], operation: ["操作", "Operation"],
};
export function displayLabel(value: string): string {
  const label = labels[value];
  return label ? t(...label) : t("未识别的事件", "Unrecognized event");
}
