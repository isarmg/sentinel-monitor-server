import { displayLabel } from "./display-labels";
import { t, getLocale } from "../shell/i18n.js";
import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import type { FormEvent } from "react";
import { createSarmgAdminApplication, errorRequestId, useAdminApplication, InstancePageNavigation, InstanceHeaderActions, InstanceWorkspace, InstanceNameField, type InstancePage } from "../shell/index.js";
import { Button, Checkbox, ConfirmDangerDialog, Dialog, ErrorState, LoadingState, Select, Table, TextField } from "@sarmg/admin-ui";

import "@sarmg/design-tokens/tokens.css";
import "@sarmg/design-tokens/tokens.dark.css";
import "../fonts/fonts.css";
import "@sarmg/admin-ui/styles.css";
import "@sarmg/design-tokens/reset.css";
import "@sarmg/design-tokens/accessibility.css";
import "./styles.css";
import "../appearance/content-blocks.css";

import {
  administratorApi,
  apiPath,
  isAuditRows,
  isCameraMutation,
  isCameras,
  isDiscoveredDevices,
  isMonitorEvents,
  isOperation,
  isRecordingSpans,
  isStreamTicket,
  isSentinelClient,
  isSentinelClients,
  isSystemStatus,
  isUndefined,
  request,
  type AuditRow,
  type Camera,
  type MonitorEvent,
  type RecordingSpan,
  type SentinelClient,
  type SystemStatus,
} from "./api";
import { WhepPlayer } from "./whep";

type View = InstancePage;
type CameraDraft = {
  id: string;
  name: string;
  location: string;
  main_stream_url: string;
  sub_stream_url: string;
  onvif_url: string;
  username: string;
  password: string;
  enabled: boolean;
  record_enabled: boolean;
};

const emptyCamera = (): CameraDraft => ({
  id: "",
  name: "",
  location: "",
  main_stream_url: "",
  sub_stream_url: "",
  onvif_url: "",
  username: "",
  password: "",
  enabled: true,
  record_enabled: true,
});
function Console() {
  const { notify } = useAdminApplication();
  const [view, setView] = useState<View>(currentView);
  useEffect(() => { const changed = () => setView(currentView()); window.addEventListener("hashchange", changed); return () => window.removeEventListener("hashchange", changed); }, []);
  const toast = useCallback((message: string, _type = "info") => notify(message), [notify]);
  const [cameras, setCameras] = useState<Camera[]>([]);
  const [events, setEvents] = useState<MonitorEvent[]>([]);
  const [audit, setAudit] = useState<AuditRow[]>([]);
  const [status, setStatus] = useState<SystemStatus | null>(null);
  const [clients, setClients] = useState<SentinelClient[]>([]);
  const [systemFailure, setSystemFailure] = useState<{ requestId?: string } | null>(null);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [unacknowledgedOnly, setUnacknowledgedOnly] = useState(false);
  const [cameraDraft, setCameraDraft] = useState<CameraDraft | null>(null);
  const [drawerCamera, setDrawerCamera] = useState<Camera | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Camera | null>(null);
  const [deletePending, setDeletePending] = useState(false);
  const deleteBusy = useRef(false);
  const [deleteFailure, setDeleteFailure] = useState<{ requestId?: string } | null>(null);
  const loadCameras = useCallback(async () => {
    setCameras(await request("/cameras", isCameras));
  }, []);
  const loadEvents = useCallback(async () => {
    const suffix = unacknowledgedOnly ? "?unacknowledged=true" : "";
    setEvents(await request(`/events${suffix}`, isMonitorEvents));
  }, [unacknowledgedOnly]);
  const loadSystem = useCallback(async () => {
    setSystemFailure(null);
    try {
      const [nextStatus, nextAudit] = await Promise.all([
        request("/system/status", isSystemStatus),
        request("/audit?limit=30", isAuditRows),
      ]);
      setStatus(nextStatus); setAudit(nextAudit);
    } catch (error) {
      setStatus(null); setAudit([]); setSystemFailure({ requestId: errorRequestId(error) });
      throw error;
    }
  }, []);
  const loadClients = useCallback(async () => {
    setClients(await request("/clients", isSentinelClients));
  }, []);

  useEffect(() => { void Promise.all([loadCameras(), loadEvents()]).catch((error) => toast(errorText(error), "error")); }, [loadCameras, loadEvents, toast]);
  useEffect(() => { if (view === "logs") void loadSystem().catch((error) => toast(errorText(error), "error")); }, [loadSystem, toast, view]);
  useEffect(() => { if (view === "instances") void loadClients().catch((error) => toast(errorText(error), "error")); }, [loadClients, toast, view]);
  useEffect(() => {
    const source = new EventSource(apiPath("/events/stream"));
    source.addEventListener("system-event", (message) => {
      try {
        const payload: unknown = JSON.parse((message as MessageEvent<string>).data);
        if (typeof payload === "object" && payload !== null && "message" in payload) {
          toast(displayLabel("kind" in payload ? String(payload.kind) : ""), "severity" in payload ? String(payload.severity) : "info");
        }
      } catch {
        toast(t("收到无法解析的事件通知", "Received an unreadable event notification"), "warning");
      }
      void Promise.all([loadCameras(), loadEvents()]).catch((error) => toast(errorText(error), "error"));
    });
    return () => source.close();
  }, [loadCameras, loadEvents, toast]);

  const online = cameras.filter((camera) => camera.status === "online").length;
  const filtered = useMemo(() => {
    const term = search.trim().toLowerCase();
    return cameras.filter((camera) => `${camera.name} ${camera.location}`.toLowerCase().includes(term));
  }, [cameras, search]);
  const chosen = filtered.find(camera => camera.id === selected) ?? filtered[0];
  const visible = chosen ? [chosen] : [];

  const saveCamera = async (draft: CameraDraft) => {
    const payload: Record<string, unknown> = {
      name: draft.name,
      location: draft.location,
      username: draft.username,
      enabled: draft.enabled,
      record_enabled: draft.record_enabled,
    };
    for (const key of ["main_stream_url", "sub_stream_url", "onvif_url", "password"] as const) {
      if (draft[key] !== "") payload[key] = draft[key];
    }
    const result = await request(draft.id === "" ? "/cameras" : `/cameras/${draft.id}`, isCameraMutation, {
      method: draft.id === "" ? "POST" : "PUT", body: JSON.stringify(payload),
    });
    setCameraDraft(null);
    toast(result.warning ? t("设备已保存，但媒体配置需要核对。", "Device saved; media configuration needs review.") : t("摄像头已保存，媒体配置正在后台应用", "Camera saved; media configuration is being applied in the background"), result.warning === null ? "success" : "warning");
    await loadCameras();
  };
  const deleteCamera = async (camera: Camera) => {
    await request(`/cameras/${camera.id}`, isOperation, { method: "DELETE" });
    toast(t("摄像头已删除", "Camera deleted"), "success"); await loadCameras();
  };
  const discover = async () => {
    const devices = await request("/discovery/onvif", isDiscoveredDevices, { method: "POST" });
    if (devices.length === 0) return toast(t("没有发现ONVIF设备；可改用手动添加", "No ONVIF devices found. Try adding a camera manually"), "warning");
    setCameraDraft({ ...emptyCamera(), onvif_url: devices[0]?.xaddrs[0] ?? "" });
    toast(t("发现 {0} 台设备，已填入第一台的ONVIF地址", "Found {0} devices; filled in the first ONVIF address", [devices.length]), "success");
  };
  const acknowledge = async (id: string) => {
    await request(`/events/${id}/ack`, isUndefined, { method: "POST" });
    await loadEvents();
  };

  return <div className="sentinel-business sarmg-content-stack">
    <InstanceHeaderActions create={view !== "logs" ? () => setCameraDraft(emptyCamera()) : undefined} createLabel={t("新建直连摄像头", "Create direct camera")} refresh={() => void Promise.all([loadCameras(), loadEvents(), ...(view === "logs" ? [loadSystem()] : []), ...(view === "instances" ? [loadClients()] : [])]).catch(error => toast(errorText(error), "error"))} />
    <InstanceWorkspace instances={filtered} selected={chosen?.id} select={id => { setSelected(id); window.location.hash = "details"; }} label={t("摄像头实例", "Camera instances")} showSidebar={view === "details"}>
    <InstancePageNavigation page={view} detailsDisabled={!chosen} navigate={value => { window.location.hash = value; }} />
    <h1 className="sarmg-visually-hidden">{viewTitle(view)}</h1><p>{online} / {cameras.length} {t("在线", "Online")}</p>
    {view === "instances" && <><InstanceOverview cameras={filtered} select={id => { setSelected(id); window.location.hash = "details"; }} search={search} setSearch={setSearch} /><ClientsView clients={clients} changed={loadClients} toast={toast} /></>}
    {view === "details" && <CameraView cameras={visible} search={search} setSearch={setSearch} edit={(camera) => setCameraDraft(toCameraDraft(camera))} remove={(camera) => { setDeleteFailure(null); setDeleteTarget(camera); }} inspect={setDrawerCamera} discover={() => void discover().catch((error) => toast(errorText(error), "error"))} />}
    {view === "logs" && <><RecordingsView cameras={cameras.filter(camera => camera.storage_mode === "server")} toast={toast} /><EventsView events={events} cameras={cameras} unacknowledgedOnly={unacknowledgedOnly} setUnacknowledgedOnly={setUnacknowledgedOnly} refresh={() => void loadEvents().catch((error) => toast(errorText(error), "error"))} acknowledge={(id) => void acknowledge(id).catch((error) => toast(errorText(error), "error"))} /><SystemView status={status} failure={systemFailure} audit={audit} refresh={() => void loadSystem().catch((error) => toast(errorText(error), "error"))} /></>}
    </InstanceWorkspace>
    {cameraDraft !== null && <CameraEditor draft={cameraDraft} setDraft={setCameraDraft} save={saveCamera} />}
    {drawerCamera !== null && <CameraDrawer camera={drawerCamera} close={() => setDrawerCamera(null)} toast={toast} />}
    {deleteTarget && <ConfirmDangerDialog title={t("删除摄像头“", "Delete camera “") + deleteTarget.name + "”？"} description={t("已有录像文件不会立即删除。", "Existing recordings will not be deleted immediately.")} pending={deletePending} onClose={() => { if (!deleteBusy.current) setDeleteTarget(null); }} onConfirm={() => {
      if (deleteBusy.current) return;
      deleteBusy.current = true; setDeletePending(true); setDeleteFailure(null);
      void deleteCamera(deleteTarget).then(() => setDeleteTarget(null))
        .catch(error => setDeleteFailure({ requestId: errorRequestId(error) }))
        .finally(() => { deleteBusy.current = false; setDeletePending(false); });
    }}>{deleteFailure && <ErrorState requestId={deleteFailure.requestId}>{t("删除未能完成，请刷新摄像头列表确认状态。", "Deletion could not be completed. Refresh the camera list to check the state.")}</ErrorState>}</ConfirmDangerDialog>}
  </div>;
}

function CameraView({ cameras, search, setSearch, edit, remove, inspect, discover }: {
  cameras: Camera[]; search: string; setSearch(value: string): void;
  edit(camera: Camera): void; remove(camera: Camera): void; inspect(camera: Camera): void; discover(): void;
}) {
  return <section className="view active sarmg-content-stack"><div className="command-bar"><div className="search-wrap"><span>⌕</span><TextField type="search" aria-label={t("搜索摄像头", "Search cameras")} value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("搜索名称或位置", "Search by name or location")} /></div><div className="command-actions"><Button className="button button-quiet" onClick={discover}>{t("发现ONVIF设备", "Discover ONVIF devices")}</Button></div></div>
    <div className="camera-grid">{cameras.length === 0 ? <div className="empty-state full-span">{t("还没有匹配的摄像头。", "No matching cameras yet.")}</div> : cameras.map((camera, index) => <article key={camera.id} className="camera-card sarmg-content-panel reveal" style={{ animationDelay: `${index * 45}ms` }}><LiveVideo camera={camera} profile={camera.has_sub_stream ? "sub" : "main"} /><div className="camera-meta"><div><span className={`status-dot ${effectiveStatus(camera)}`} /><strong>{camera.name}</strong><small>{camera.location || t("未标注位置", "Location not specified")} · {[camera.manufacturer, camera.model].filter(Boolean).join(" ") || camera.adapter_kind.toUpperCase()} · {camera.storage_mode === "server" ? t("服务器录像", "Server recording") : t("客户端录像", "Client recording")}</small></div><span className="camera-status">{statusLabel(effectiveStatus(camera))}</span></div><div className="camera-actions"><Button onClick={() => inspect(camera)}>{t("主码流", "Main stream")}</Button>{camera.source_kind === "direct" && <><Button onClick={() => edit(camera)}>{t("配置", "Configuration")}</Button><Button className="danger-link" onClick={() => remove(camera)}>{t("删除", "Delete")}</Button></>}</div></article>)}</div>
    </section>;
}

function InstanceOverview({ cameras, select, search, setSearch }: { cameras: Camera[]; select(id: string): void; search: string; setSearch(value: string): void }) {
  return <section className="view active sarmg-content-stack"><h2>{t("实例总览", "Instance overview")}</h2><TextField type="search" aria-label={t("搜索摄像头", "Search cameras")} value={search} onChange={event => setSearch(event.target.value)} placeholder={t("搜索名称或位置", "Search by name or location")} />
    <Table aria-label={t("摄像头实例列表", "Camera instance list")}><thead><tr><th>{t("名称", "Name")}</th><th>{t("厂商与型号", "Make and model")}</th><th>{t("接入方式", "Adapter")}</th><th>{t("状态", "Status")}</th><th>{t("存储", "Storage")}</th></tr></thead><tbody>{cameras.map(camera => <tr key={camera.id}><th><Button onClick={() => select(camera.id)}>{camera.name}</Button></th><td>{[camera.manufacturer, camera.model].filter(Boolean).join(" ") || "—"}</td><td>{camera.adapter_kind.toUpperCase()}</td><td>{statusLabel(effectiveStatus(camera))}</td><td>{camera.storage_mode === "server" ? t("服务器", "Server") : t("客户端", "Client")}</td></tr>)}</tbody></Table>
  </section>;
}

function LiveVideo({ camera, profile, controls = false }: { camera: Camera; profile: string; controls?: boolean }) {
  const video = useRef<HTMLVideoElement>(null);
  const [label, setLabel] = useState(camera.enabled ? t("正在连接", "Connecting") : t("设备已停用", "Device disabled"));
  useEffect(() => {
    const element = video.current;
    if (element === null || !camera.enabled) return;
    let closed = false;
    let close: (() => void) | undefined;
    void request(`/cameras/${camera.id}/stream-ticket?profile=${profile}`, isStreamTicket).then(async (ticket) => {
      if (closed) return;
      const whep = new WhepPlayer(element, ticket.whep_url, ticket.token); close = () => whep.close();
      try { await whep.start(); if (!closed) setLabel(""); }
      catch {
        whep.close();
        const retry = await request(`/cameras/${camera.id}/stream-ticket?profile=${profile}`, isStreamTicket);
        if (closed) return;
        const { default: Hls } = await import("hls.js");
        if (closed) return;
        const hls = new Hls({ lowLatencyMode: true, xhrSetup: (xhr) => xhr.setRequestHeader("Authorization", `Bearer ${retry.token}`) });
        hls.loadSource(retry.hls_url); hls.attachMedia(element); close = () => hls.destroy();
        hls.on(Hls.Events.MANIFEST_PARSED, () => { void element.play().catch(() => { if (!closed) setLabel(t("请点击播放", "Click to play")); }); if (!closed) setLabel(""); });
        hls.on(Hls.Events.ERROR, (_event, data) => { if (data.fatal) setLabel(t("视频暂不可用", "Video temporarily unavailable")); });
      }
    }).catch((error) => { if (!closed) setLabel(errorText(error)); });
    return () => { closed = true; close?.(); element.removeAttribute("src"); element.srcObject = null; };
  }, [camera.enabled, camera.id, profile]);
  return <div className={controls ? "detail-video" : "video-shell"}><video ref={video} muted autoPlay playsInline controls={controls} />{label !== "" && <div className="video-state">{label}</div>}{!controls && <div className="scanline" />}</div>;
}

function ClientsView({ clients, changed, toast }: {
  clients: SentinelClient[];
  changed(): Promise<void>;
  toast(message: string, type?: string): void;
}) {
  const [name, setName] = useState("");
  const [pending, setPending] = useState(false);
  const [rotating, setRotating] = useState<SentinelClient | null>(null);
  const [removing, setRemoving] = useState<SentinelClient | null>(null);
  const randomCode = () => {
    const bytes = crypto.getRandomValues(new Uint8Array(32));
    return Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
  };
  const create = async () => {
    setPending(true);
    try {
      await request("/clients", isSentinelClient, { method: "POST", body: JSON.stringify({ name }) });
      setName(""); await changed();
      toast(t("客户端实例已创建", "Client instance created"), "success");
    } finally { setPending(false); }
  };
  const rotate = async (client: SentinelClient) => {
    setPending(true);
    try {
      await request(`/clients/${client.id}/authorization`, isSentinelClient, { method: "PUT", body: JSON.stringify({ authorization_code: randomCode() }) });
      await changed();
      toast(t("实例授权码已更换；原客户端必须使用新码重新配对", "Instance authorization code changed; the old client must pair again with the new code"), "success");
    } finally { setPending(false); }
  };
  const remove = async (client: SentinelClient) => {
    setPending(true);
    try {
      await request(`/clients/${client.id}`, (value): value is undefined => value === undefined, { method: "DELETE" });
      await changed();
      toast(client.status === "revoked" ? t("实例已删除", "Instance deleted") : t("实例已取消或撤销，可再次删除其信息", "Instance cancelled or revoked; delete it again to remove its entry"), "success");
    } finally { setPending(false); }
  };
  return <section className="view active sarmg-content-stack">
    <section className="management-block sarmg-content-panel"><div className="section-heading"><h2>{t("新建客户端实例", "Create client instance")}</h2></div>
      <p>{t("每个实例拥有一个长期授权码。服务端加密保存并可查看；更换授权码会撤销现有客户端，必须重新配对。", "Each instance has one long-lived authorization code. It is encrypted, remains visible on the Server, and changing it revokes the current client until it pairs again.")}</p>
      <label>{t("实例名称", "Instance name")}<TextField value={name} onChange={event => setName(event.target.value)} /></label>
      <div className="sarmg-actions"><Button disabled={pending || name.trim() === ""} onClick={() => void create().catch(error => toast(errorText(error), "error"))}>{pending ? t("创建中…", "Creating…") : t("创建实例", "Create instance")}</Button></div>
    </section>
    <Table aria-label={t("客户端实例", "Client instances")}><thead><tr><th>{t("名称", "Name")}</th><th>{t("状态", "Status")}</th><th>{t("永久授权码", "Permanent authorization code")}</th><th>{t("版本", "Version")}</th><th>{t("操作", "Actions")}</th></tr></thead><tbody>
      {clients.length === 0 ? <tr><td colSpan={5} className="empty-state">{t("尚无客户端实例", "No client instances")}</td></tr> : clients.map(client => <tr key={client.id}><td>{client.name}</td><td>{displayLabel(client.status)}</td><td><code>{client.authorization_code}</code></td><td>{client.client_version ?? "—"}</td><td>{client.status !== "revoked" && <Button disabled={pending} onClick={() => setRotating(client)}>{t("更换授权码", "Change code")}</Button>}<Button disabled={pending} onClick={() => setRemoving(client)}>{client.status === "pending" ? t("取消配对", "Cancel pairing") : client.status === "revoked" ? t("删除实例", "Delete instance") : t("撤销实例", "Revoke instance")}</Button></td></tr>)}
    </tbody></Table>
    {rotating && <ConfirmDangerDialog title={t("更换实例授权码", "Change instance authorization code")} description={t("这会立即撤销当前客户端凭据并停用它管理的摄像头。客户端必须使用新授权码重新配对并重新上报摄像头。", "This immediately revokes the current client credential and disables its cameras. The client must pair again with the new authorization code and report its cameras again.")} pending={pending} onClose={() => { if (!pending) setRotating(null); }} onConfirm={() => { const target = rotating; setRotating(null); void rotate(target).catch(error => toast(errorText(error), "error")); }} />}
    {removing && <ConfirmDangerDialog title={removing.status === "revoked" ? t("删除实例", "Delete instance") : removing.status === "pending" ? t("取消配对", "Cancel pairing") : t("撤销实例", "Revoke instance")} description={removing.status === "revoked" ? t("永久删除已取消或撤销的实例信息；若仍关联摄像头，请先删除摄像头。", "Permanently delete the cancelled or revoked instance entry. Delete any linked cameras first.") : t("当前授权码和客户端凭据将失效；操作完成后仍可从列表永久删除该实例信息。", "The authorization code and client credential will be invalidated. The instance entry can then be permanently deleted from the list.")} pending={pending} onClose={() => { if (!pending) setRemoving(null); }} onConfirm={() => { const target = removing; setRemoving(null); void remove(target).catch(error => toast(errorText(error), "error")); }} />}
  </section>;
}

function RecordingsView({ cameras, toast }: { cameras: Camera[]; toast(message: string, type?: string): void }) {
  const [cameraId, setCameraId] = useState(cameras[0]?.id ?? "");
  const [start, setStart] = useState(() => localDateInput(new Date(Date.now() - 86_400_000)));
  const [end, setEnd] = useState(() => localDateInput(new Date()));
  const [spans, setSpans] = useState<RecordingSpan[]>([]);
  const [playing, setPlaying] = useState<RecordingSpan | null>(null);
  useEffect(() => { if (cameraId === "" && cameras[0] !== undefined) setCameraId(cameras[0].id); }, [cameraId, cameras]);
  const search = async () => {
    if (cameraId === "") return toast(t("请先添加摄像头", "Add a camera first"), "warning");
    const query = new URLSearchParams({ camera_id: cameraId, start: new Date(start).toISOString(), end: new Date(end).toISOString() });
    setSpans(await request(`/recordings?${query}`, isRecordingSpans));
  };
  const playback = playing === null ? "" : apiPath(`/recordings/play?${new URLSearchParams({ camera_id: cameraId, start: playing.start, duration: String(playing.duration), format: "mp4" })}`);
  return <section className="view active sarmg-content-stack"><div className="filter-panel sarmg-content-panel"><label>{t("摄像头", "Camera")}<Select value={cameraId} onChange={(event) => setCameraId(event.target.value)}>{cameras.map((camera) => <option key={camera.id} value={camera.id}>{camera.name}</option>)}</Select></label><label>{t("开始时间", "Start time")}<TextField type="datetime-local" value={start} onChange={(event) => setStart(event.target.value)} /></label><label>{t("结束时间", "End time")}<TextField type="datetime-local" value={end} onChange={(event) => setEnd(event.target.value)} /></label><Button className="button button-primary" onClick={() => void search().catch((error) => toast(errorText(error), "error"))}>{t("查询录像", "Search recordings")}</Button></div><div className="recording-layout sarmg-content-panel"><div><div className="section-heading"><h3>{t("录像时间段", "Recording segments")}</h3><span>{spans.length} {t("条", "segments")}</span></div><div className="record-list">{spans.length === 0 ? <div className="empty-state">{t("所选范围内没有录像", "No recordings in the selected range")}</div> : spans.map((span) => <Button key={`${span.start}-${span.duration}`} className="record-item" onClick={() => setPlaying(span)}><span>{formatDate(span.start)}</span><strong>{formatDuration(span.duration)}</strong><i>{t("播放", "Play")}</i></Button>)}</div></div><div className="playback-stage"><video src={playback || undefined} controls playsInline autoPlay /><div>{playing === null ? t("尚未选择录像", "No recording selected") : `${formatDate(playing.start)} · ${formatDuration(playing.duration)}`}</div></div></div></section>;
}

function EventsView({ events, cameras, unacknowledgedOnly, setUnacknowledgedOnly, refresh, acknowledge }: { events: MonitorEvent[]; cameras: Camera[]; unacknowledgedOnly: boolean; setUnacknowledgedOnly(value: boolean): void; refresh(): void; acknowledge(id: string): void }) {
  const names = new Map(cameras.map((camera) => [camera.id, camera.name]));
  return <section className="view active sarmg-content-stack"><div className="command-bar"><label className="toggle-line"><Checkbox checked={unacknowledgedOnly} onChange={(event) => setUnacknowledgedOnly(event.target.checked)} />{t("仅显示未确认事件", "Show unacknowledged events only")}</label></div><Table aria-label={t("监控事件", "Monitoring events")}><thead><tr><th>{t("等级", "Severity")}</th><th>{t("事件", "Event")}</th><th>{t("摄像头", "Camera")}</th><th>{t("时间", "Time")}</th><th>{t("状态", "Status")}</th></tr></thead><tbody>{events.length === 0 ? <tr><td colSpan={5} className="empty-state">{t("没有事件", "No events")}</td></tr> : events.map((event) => <tr key={event.id}><td><span className={`severity ${event.severity}`}>{severityLabel(event.severity)}</span></td><td><strong>{displayLabel(event.kind)}</strong></td><td>{event.camera_id === null ? t("系统", "System") : names.get(event.camera_id) ?? t("系统", "System")}</td><td>{formatDate(event.created_at)}</td><td>{event.acknowledged_at === null ? <Button className="text-button" onClick={() => acknowledge(event.id)}>{t("确认", "Acknowledge")}</Button> : t("已确认", "Acknowledged")}</td></tr>)}</tbody></Table></section>;
}

function SystemView({ status, failure, audit, refresh }: { status: SystemStatus | null; failure: { requestId?: string } | null; audit: AuditRow[]; refresh(): void }) {
  return <section className="view active sarmg-content-stack">
    {failure ? <ErrorState requestId={failure.requestId} onRetry={refresh}>{t("系统状态与业务审计暂不可用。", "System status and business audit are temporarily unavailable.")}</ErrorState>
      : status === null ? <LoadingState>{t("正在加载系统状态…", "Loading system status…")}</LoadingState> : <Table aria-label={t("系统状态", "System status")}>
      <thead><tr><th scope="col">{t("项目", "Item")}</th><th scope="col">{t("当前状态", "Current status")}</th><th scope="col">{t("说明", "Description")}</th></tr></thead>
      <tbody>
        <tr><th scope="row">{t("媒体服务", "Media service")}</th><td>{status.media_service === "ok" ? t("运行正常", "Healthy") : t("连接失败", "Connection failed")}</td><td>MediaMTX</td></tr>
        <tr><th scope="row">{t("在线设备", "Online devices")}</th><td>{status.cameras.online} / {status.cameras.total}</td><td>{t("当前主码流状态", "Current main stream status")}</td></tr>
        <tr><th scope="row">{t("录像任务", "Recording tasks")}</th><td>{status.cameras.recording}</td><td>{t("主码流持续录制", "Continuous main stream recording")}</td></tr>
      </tbody>
    </Table>}
    <section className="management-block sarmg-content-panel"><div className="section-heading"><h2>{t("最近业务审计记录", "Recent business audit")}</h2></div>
      <div className="audit-list">{audit.length === 0 ? <div className="empty-state">{t("暂无审计记录", "No audit records yet")}</div> : audit.map((row) => <div key={row.id}><span>{displayLabel(row.action)}</span><small>{formatDate(row.created_at)}</small><code>{displayLabel(row.entity_type)}{row.entity_id === null ? "" : " / " + row.entity_id.slice(0, 8)}</code></div>)}</div>
    </section>
  </section>;
}

function CameraEditor({ draft, setDraft, save }: { draft: CameraDraft; setDraft(value: CameraDraft | null): void; save(value: CameraDraft): Promise<void> }) {
  const [pending, setPending] = useState(false);
  const busy = useRef(false);
  const [failure, setFailure] = useState<{ requestId?: string } | null>(null);
  const field = (key: keyof CameraDraft) => (event: React.ChangeEvent<HTMLInputElement>) => setDraft({ ...draft, [key]: event.target.type === "checkbox" ? event.target.checked : event.target.value });
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (busy.current) return;
    busy.current = true; setPending(true); setFailure(null);
    try { await save(draft); } catch (error) { setFailure({ requestId: errorRequestId(error) }); setDraft({ ...draft, password: "" }); }
    finally { busy.current = false; setPending(false); }
  }
  return <Dialog title={draft.id === "" ? t("添加摄像头", "Add camera") : t("编辑摄像头", "Edit camera")} onClose={() => { if (!busy.current) setDraft(null); }}>
    <form className="sentinel-business" onSubmit={event => void submit(event)} aria-busy={pending}>
      {failure && <ErrorState requestId={failure.requestId}>{t("设备配置未能保存，请检查输入并重试。", "Unable to save the device configuration. Check the input and retry.")}</ErrorState>}
      <fieldset disabled={pending}><div className="form-grid">
        <label>{t("名称", "Name")}<InstanceNameField value={draft.name} onChange={field("name")} required /></label>
        <label>{t("位置", "Location")}<TextField value={draft.location} onChange={field("location")} /></label>
        <label className="wide">{t("主码流 RTSP", "Main stream RTSP")}<TextField value={draft.main_stream_url} onChange={field("main_stream_url")} required={draft.id === ""} /></label>
        <label className="wide">{t("子码流 RTSP", "Sub-stream RTSP")}<TextField value={draft.sub_stream_url} onChange={field("sub_stream_url")} /></label>
        <label className="wide">{t("ONVIF设备服务地址", "ONVIF device service URL")}<TextField value={draft.onvif_url} onChange={field("onvif_url")} /></label>
        <label>{t("设备用户名", "Device username")}<TextField value={draft.username} onChange={field("username")} autoComplete="off" /></label>
        <label>{t("设备密码", "Device password")}<TextField type="password" value={draft.password} onChange={field("password")} autoComplete="new-password" /></label>
      </div><p className="field-help">{t("编辑时流地址和密码留空会保持原值；凭据不会返回浏览器。", "Leave stream URLs and passwords blank to preserve them when editing. Credentials are never returned to the browser.")}</p>
      <div className="check-row"><label><Checkbox checked={draft.enabled} onChange={field("enabled")} />{t("启用设备", "Enable device")}</label><label><Checkbox checked={draft.record_enabled} onChange={field("record_enabled")} />{t("录制主码流", "Record main stream")}</label></div>
      <div className="sarmg-actions"><Button onClick={() => setDraft(null)}>{t("取消", "Cancel")}</Button><Button type="submit">{pending ? t("正在保存…", "Saving…") : t("保存设备", "Save device")}</Button></div></fieldset>
    </form>
  </Dialog>;
}

function CameraDrawer({ camera, close, toast }: { camera: Camera; close(): void; toast(message: string, type?: string): void }) {
  const moving = useRef(false);
  const busy = useRef(false);
  const tail = useRef<Promise<void>>(Promise.resolve());
  const send = useCallback((action: "move" | "stop", vector = "0,0,0") => {
    const [pan = 0, tilt = 0, zoom = 0] = vector.split(",").map(Number);
    return request(`/cameras/${camera.id}/ptz`, isUndefined, { method: "POST", body: JSON.stringify({ action, pan, tilt, zoom }) });
  }, [camera.id]);
  const stop = useCallback(() => {
    if (!moving.current) return;
    moving.current = false;
    tail.current = tail.current.then(() => send("stop"))
      .catch(error => toast(errorText(error), "error"))
      .finally(() => { busy.current = false; });
  }, [send, toast]);
  const start = (vector: string) => {
    if (busy.current) return;
    busy.current = true; moving.current = true;
    tail.current = send("move", vector).catch(error => { toast(errorText(error), "error"); });
  };
  useEffect(() => {
    const hidden = () => { if (document.hidden) stop(); };
    window.addEventListener("blur", stop); document.addEventListener("visibilitychange", hidden);
    return () => { stop(); window.removeEventListener("blur", stop); document.removeEventListener("visibilitychange", hidden); };
  }, [stop]);
  const movement = (vector: string, label: string, glyph: string) => <Button aria-label={label}
    onPointerDown={event => { if (event.button !== 0) return; event.currentTarget.setPointerCapture(event.pointerId); start(vector); }}
    onPointerUp={stop} onPointerCancel={stop} onLostPointerCapture={stop} onBlur={stop}
    onKeyDown={event => { if (event.key === " " || event.key === "Enter") { event.preventDefault(); if (!event.repeat) start(vector); } }}
    onKeyUp={event => { if (event.key === " " || event.key === "Enter") { event.preventDefault(); stop(); } }}>{glyph}</Button>;
  return <Dialog title={camera.name} description={camera.location || t("未标注位置", "Location not specified")} onClose={() => { stop(); close(); }}>
    <div className="sentinel-business"><LiveVideo camera={camera} profile="main" controls />
      <section className="sarmg-content-panel"><h3>{t("设备信息", "Device information")}</h3><p>{[camera.manufacturer, camera.model, camera.firmware_version].filter(Boolean).join(" · ") || t("设备未报告厂商信息", "The device did not report make information")}</p><p>{t("适配器", "Adapter")}: {camera.adapter_kind.toUpperCase()} · {t("能力", "Capabilities")}: {capabilityLabels(camera).join(" / ")}</p>{camera.health_message && <p>{t("健康状态", "Health")}: {camera.health_message}</p>}</section>
      {camera.capabilities.ptz && <section className="ptz-panel" aria-label={t("云台控制", "PTZ controls")}><h3>{t("云台控制", "PTZ controls")}</h3><p>{t("按住方向键或用空格、回车启动移动，松开即停止。窗口失焦也会发送停止。", "Hold a direction button, Space or Enter to move; release to stop. Losing window focus also sends a stop command.")}</p>
        <div className="ptz-grid"><span />{movement("0,0.55,0", t("云台向上", "Tilt up"), "↑")}<span />
          {movement("-0.55,0,0", t("云台向左", "Pan left"), "←")}<Button aria-label={t("停止云台", "Stop movement")} onClick={() => {
            if (moving.current) stop();
            else if (!busy.current) { busy.current = true; tail.current = send("stop").catch(error => toast(errorText(error), "error")).finally(() => { busy.current = false; }); }
          }}>■</Button>{movement("0.55,0,0", t("云台向右", "Pan right"), "→")}<span />{movement("0,-0.55,0", t("云台向下", "Tilt down"), "↓")}<span />
        </div><div className="zoom-row">{movement("0,0,-0.45", t("云台拉远", "Zoom out"), t("− 拉远", "− Zoom out"))}{movement("0,0,0.45", t("云台拉近", "Zoom in"), t("＋ 拉近", "+ Zoom in"))}</div>
      </section>}
    </div>
  </Dialog>;
}


const viewTitle = (view: View) => ({ instances: t("实例列表", "Instance list"), details: t("详细信息", "Details"), logs: t("日志", "Logs") })[view];
const statusLabel = (status: string) => ({ pending: t("等待检测", "Waiting for detection"), online: t("在线", "Online"), offline: t("离线", "Offline"), disabled: t("已停用", "Disabled"), error: t("配置异常", "Configuration error") } as Record<string, string>)[status] ?? t("未知", "Unknown");
const severityLabel = (severity: string) => ({ info: t("信息", "Information"), warning: t("警告", "Warning"), critical: t("严重", "Critical") } as Record<string, string>)[severity] ?? t("未知", "Unknown");
const effectiveStatus = (camera: Camera) => ["error", "offline", "disabled"].includes(camera.device_status) ? camera.device_status : camera.status;
const capabilityLabels = (camera: Camera) => [camera.capabilities.video && t("视频", "Video"), camera.capabilities.sub_stream && t("子码流", "Sub-stream"), camera.capabilities.ptz && "PTZ", camera.capabilities.events && t("事件", "Events"), camera.capabilities.audio_input && t("音频", "Audio")].filter((value): value is string => Boolean(value));
const formatDate = (value: string) => new Intl.DateTimeFormat(getLocale(), { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }).format(new Date(value));
const formatDuration = (seconds: number) => { const total = Math.round(seconds); const hours = Math.floor(total / 3600); const minutes = Math.floor((total % 3600) / 60); return [hours > 0 ? t("{0}时", "{0}h", [hours]) : "", minutes > 0 ? t("{0}分", "{0}m", [minutes]) : "", t("{0}秒", "{0}s", [total % 60]) ].filter(Boolean).join(getLocale() === "en" ? " " : ""); };
const localDateInput = (date: Date) => new Date(date.getTime() - date.getTimezoneOffset() * 60_000).toISOString().slice(0, 16);
const errorText = (error: unknown) => { const id = errorRequestId(error); return id ? t("请求失败，请重试。请求标识： {0}", "Request failed. Please retry. Request ID: {0}", [id]) : t("请求失败，请重试。", "Request failed. Please retry."); };
const toCameraDraft = (camera: Camera): CameraDraft => ({ ...emptyCamera(), id: camera.id, name: camera.name, location: camera.location, username: camera.username ?? "", enabled: camera.enabled, record_enabled: camera.record_enabled });

function currentView(): View {
  const hash = window.location.hash.slice(1);
  return hash === "details" || hash === "logs" ? hash : "instances";
}
const Root = createSarmgAdminApplication({
  product: { name: "Sentinel Monitor" }, client: administratorApi,
  navigation: [],
  routes: <Console />,
});
const root = document.getElementById("root");
if (root === null) throw new Error("缺少React根节点");
createRoot(root).render(<StrictMode><Root /></StrictMode>);
