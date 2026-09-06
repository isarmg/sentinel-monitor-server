import { t } from "../shell/i18n.js";
const labels: Record<string, readonly [string, string]> = {
  "camera.online": ["摄像头已恢复在线", "Camera is back online"], "camera.offline": ["摄像头已离线", "Camera is offline"],
  "camera.status": ["摄像头状态变更", "Camera status changed"], "camera.updated": ["更新摄像头", "Update camera"],
  "camera.create": ["创建摄像头", "Create camera"], "camera.update": ["更新摄像头", "Update camera"],
  "camera.delete": ["删除摄像头", "Delete camera"], "camera.ptz": ["控制云台", "Control PTZ"],
  "event.acknowledge": ["确认事件", "Acknowledge event"], "event.ack": ["确认事件", "Acknowledge event"],
  "media.operation.resolve": ["核对媒体操作", "Reconcile media operation"],
  camera: ["摄像头", "Camera"], event: ["事件", "Event"], operation: ["操作", "Operation"],
};
export function displayLabel(value: string): string {
  const label = labels[value];
  return label ? t(...label) : t("未识别的事件", "Unrecognized event");
}
