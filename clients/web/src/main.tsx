import { displayLabel } from "./display-labels";
import { t, getLocale } from "@sarmg/admin-ui/i18n";
import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import type { FormEvent } from "react";
import { createSarmgAdminApplication, errorRequestId, useAdminApplication, InstancePageNavigation, InstanceHeaderActions, InstanceWorkspace, InstanceNameField, type InstancePage } from "@sarmg/admin-shell";
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
  isCameras,
  isMonitorEvents,
  isRecordingSpans,
  isStreamTicket,
  isSentinelClient,
  isSentinelClients,
  isUndefined,
  request,
  type AuditRow,
  type Camera,
  type MonitorEvent,
  type RecordingSpan,
  type SentinelClient,
} from "./api";
import { WhepPlayer } from "./whep";

type View = InstancePage;
function Console() {
  const { notify } = useAdminApplication();
  const [view, setView] = useState<View>(currentView);
  useEffect(() => { const changed = () => setView(currentView()); window.addEventListener("hashchange", changed); return () => window.removeEventListener("hashchange", changed); }, []);
  const toast = useCallback((message: string, _type = "info") => notify(message), [notify]);
  const [cameras, setCameras] = useState<Camera[]>([]);
  const [events, setEvents] = useState<MonitorEvent[]>([]);
  const [audit, setAudit] = useState<AuditRow[] | null>(null);
  const [clients, setClients] = useState<SentinelClient[]>([]);
  const [auditFailure, setAuditFailure] = useState<{ requestId?: string } | null>(null);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [unacknowledgedOnly, setUnacknowledgedOnly] = useState(false);
  const [creatingClient, setCreatingClient] = useState(false);
  const [drawerCamera, setDrawerCamera] = useState<Camera | null>(null);
  const loadCameras = useCallback(async () => {
    setCameras(await request("/cameras", isCameras));
  }, []);
  const loadEvents = useCallback(async () => {
    const suffix = unacknowledgedOnly ? "?unacknowledged=true" : "";
    setEvents(await request(`/events${suffix}`, isMonitorEvents));
  }, [unacknowledgedOnly]);
  const loadAudit = useCallback(async () => {
    setAudit(null);
    setAuditFailure(null);
    try {
      setAudit(await request("/audit?limit=30", isAuditRows));
    } catch (error) {
      setAudit(null); setAuditFailure({ requestId: errorRequestId(error) });
      throw error;
    }
  }, []);
  const loadClients = useCallback(async () => {
    setClients(await request("/clients", isSentinelClients));
  }, []);

  useEffect(() => { void Promise.all([loadCameras(), loadEvents(), loadClients()]).catch((error) => toast(errorText(error), "error")); }, [loadCameras, loadClients, loadEvents, toast]);
  useEffect(() => { if (view === "logs") void loadAudit().catch((error) => toast(errorText(error), "error")); }, [loadAudit, toast, view]);
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

  const filteredCameras = useMemo(() => {
    const term = search.trim().toLowerCase();
    return cameras.filter((camera) => `${camera.name} ${camera.location}`.toLowerCase().includes(term));
  }, [cameras, search]);
  const chosen = clients.find(client => client.id === selected) ?? clients[0];
  const clientCameras = chosen === undefined ? [] : cameras.filter(camera => camera.client_id === chosen.id);
  const visible = chosen === undefined ? [] : filteredCameras.filter(camera => camera.client_id === chosen.id);
  const recordings = clientCameras.filter(camera => camera.storage_mode === "server");
  const acknowledge = async (id: string) => {
    await request(`/events/${id}/ack`, isUndefined, { method: "POST" });
    await loadEvents();
  };

  return <div className="sentinel-business sarmg-content-stack">
    <InstanceHeaderActions create={() => setCreatingClient(true)} refresh={() => void Promise.all([loadCameras(), loadEvents(), loadClients(), ...(view === "logs" ? [loadAudit()] : [])]).catch(error => toast(errorText(error), "error"))} />
    <InstanceWorkspace instances={clients} selected={chosen?.id} select={id => { setSelected(id); window.location.hash = "details"; }} label={t("摄像机实例", "Camera instances")} showSidebar={view === "details"}>
    <InstancePageNavigation page={view} detailsDisabled={!chosen} navigate={value => { window.location.hash = value; }} />
    <h1 className="sarmg-visually-hidden">{viewTitle(view)}</h1>
    {view === "instances" && <><InstanceStatistics cameras={cameras} clients={clients} /><section className="view active sarmg-content-stack"><h2>{t("实例列表", "Instance list")}</h2><ClientsView clients={clients} cameras={cameras} changed={loadClients} toast={toast} select={id => { setSelected(id); window.location.hash = "details"; }} /></section></>}
    {view === "details" && chosen && <><ClientDetails client={chosen} cameras={clientCameras} /><CameraView cameras={visible} search={search} setSearch={setSearch} inspect={setDrawerCamera} />{recordings.length > 0 && <RecordingsView cameras={recordings} toast={toast} />}</>}
    {view === "logs" && <><EventsView events={events} cameras={cameras} unacknowledgedOnly={unacknowledgedOnly} setUnacknowledgedOnly={setUnacknowledgedOnly} refresh={() => void loadEvents().catch((error) => toast(errorText(error), "error"))} acknowledge={(id) => void acknowledge(id).catch((error) => toast(errorText(error), "error"))} /><AuditLogView failure={auditFailure} audit={audit} refresh={() => void loadAudit().catch((error) => toast(errorText(error), "error"))} /></>}
    </InstanceWorkspace>
    {creatingClient && <CreateClientDialog close={() => setCreatingClient(false)} changed={loadClients} toast={toast} />}
    {drawerCamera !== null && <CameraDrawer camera={drawerCamera} close={() => setDrawerCamera(null)} toast={toast} />}
  </div>;
}

function CameraView({ cameras, search, setSearch, inspect }: {
  cameras: Camera[]; search: string; setSearch(value: string): void;
  inspect(camera: Camera): void;
}) {
  return <section className="view active sarmg-content-stack"><div className="command-bar"><div className="search-wrap"><span>⌕</span><TextField type="search" aria-label={t("搜索摄像头", "Search cameras")} value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("搜索名称或位置", "Search by name or location")} /></div></div>
    <div className="camera-grid">{cameras.length === 0 ? <div className="empty-state full-span">{t("该客户端还没有上报匹配的摄像头。", "This client has not reported any matching cameras yet.")}</div> : cameras.map((camera, index) => <article key={camera.id} className="camera-card sarmg-content-panel reveal" style={{ animationDelay: `${index * 45}ms` }}><LiveVideo camera={camera} profile={camera.has_sub_stream ? "sub" : "main"} /><div className="camera-meta"><div><span className={`status-dot ${effectiveStatus(camera)}`} /><strong>{camera.name}</strong><small>{camera.location || t("未标注位置", "Location not specified")} · {[camera.manufacturer, camera.model].filter(Boolean).join(" ") || camera.adapter_kind.toUpperCase()} · {camera.storage_mode === "server" ? t("服务器录像", "Server recording") : t("客户端录像", "Client recording")}</small></div><span className="camera-status">{statusLabel(effectiveStatus(camera))}</span></div><div className="camera-actions"><Button onClick={() => inspect(camera)}>{t("查看与控制", "View and control")}</Button></div></article>)}</div>
    </section>;
}

function InstanceStatistics({ cameras, clients }: { cameras: Camera[]; clients: SentinelClient[] }) {
  return <section className="view active sarmg-content-stack"><h2>{t("统计", "Statistics")}</h2><Table aria-label={t("实例统计", "Instance statistics")}><thead><tr><th>{t("统计项", "Metric")}</th><th>{t("当前值", "Current value")}</th></tr></thead><tbody>
    <tr><th scope="row">{t("实例总数", "Total instances")}</th><td>{clients.length}</td></tr><tr><th scope="row">{t("在线实例", "Online instances")}</th><td>{clients.filter(client => client.status === "online").length}</td></tr><tr><th scope="row">{t("待配对实例", "Instances awaiting pairing")}</th><td>{clients.filter(client => client.status === "pending").length}</td></tr><tr><th scope="row">{t("已上报摄像机", "Reported cameras")}</th><td>{cameras.filter(camera => camera.source_kind === "client").length}</td></tr>
  </tbody></Table></section>;
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

function ClientsView({ clients, cameras, changed, toast, select }: {
  clients: SentinelClient[];
  cameras: Camera[];
  changed(): Promise<void>;
  toast(message: string, type?: string): void;
  select(id: string): void;
}) {
  const [pending, setPending] = useState(false);
  const [rotating, setRotating] = useState<SentinelClient | null>(null);
  const [removing, setRemoving] = useState<SentinelClient | null>(null);
  const randomCode = () => {
    const bytes = crypto.getRandomValues(new Uint8Array(32));
    return Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
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
  const cameraByInstance = new Map(cameras.filter(camera => camera.client_id !== null).map(camera => [camera.client_id, camera]));
  return <div className="sarmg-content-stack">
    <p>{t("一个实例对应一台摄像机和一个永久授权码；同一客户端安装可使用多个授权码分别连接多台摄像机。", "Each instance represents one camera and one permanent authorization code. One client installation may use multiple codes for separate cameras.")}</p>
    <Table aria-label={t("摄像机实例列表", "Camera instance list")}><thead><tr><th>{t("名称", "Name")}</th><th>{t("配对状态", "Pairing status")}</th><th>{t("摄像机状态", "Camera status")}</th><th>{t("厂商 / 型号", "Make / model")}</th><th>{t("永久授权码", "Permanent authorization code")}</th><th>{t("操作", "Actions")}</th></tr></thead><tbody>
      {clients.length === 0 ? <tr><td colSpan={6} className="empty-state">{t("尚无摄像机实例", "No camera instances")}</td></tr> : clients.map(client => { const camera = cameraByInstance.get(client.id); return <tr key={client.id}><th scope="row"><Button onClick={() => select(client.id)}>{client.name}</Button></th><td>{displayLabel(client.status)}</td><td>{camera ? statusLabel(effectiveStatus(camera)) : t("尚未上报", "Not yet reported")}</td><td>{camera ? [camera.manufacturer, camera.model].filter(Boolean).join(" / ") || camera.adapter_kind.toUpperCase() : "—"}</td><td><code>{client.authorization_code}</code></td><td>{client.status !== "revoked" && <Button disabled={pending} onClick={() => setRotating(client)}>{t("更换授权码", "Change code")}</Button>}<Button disabled={pending} onClick={() => setRemoving(client)}>{client.status === "pending" ? t("取消配对", "Cancel pairing") : client.status === "revoked" ? t("删除实例", "Delete instance") : t("撤销实例", "Revoke instance")}</Button></td></tr>; })}
    </tbody></Table>
    {rotating && <ConfirmDangerDialog title={t("更换实例授权码", "Change instance authorization code")} description={t("这会立即撤销当前客户端凭据并停用它管理的摄像头。客户端必须使用新授权码重新配对并重新上报摄像头。", "This immediately revokes the current client credential and disables its cameras. The client must pair again with the new authorization code and report its cameras again.")} pending={pending} onClose={() => { if (!pending) setRotating(null); }} onConfirm={() => { const target = rotating; setRotating(null); void rotate(target).catch(error => toast(errorText(error), "error")); }} />}
    {removing && <ConfirmDangerDialog title={removing.status === "revoked" ? t("删除实例", "Delete instance") : removing.status === "pending" ? t("取消配对", "Cancel pairing") : t("撤销实例", "Revoke instance")} description={removing.status === "revoked" ? t("媒体路径清理确认完成后，永久删除该摄像机状态和授权实例；若清理尚未收敛，服务端会拒绝并要求稍后重试。", "Permanently delete the camera state and authorization instance after media-path removal is confirmed. If cleanup has not converged, the server rejects the request and asks you to retry later.") : t("当前授权码和客户端凭据将失效；服务端会先清理媒体路径，完成后可再永久删除该实例。", "The authorization code and client credential will be invalidated. The server first removes media paths; after cleanup, the instance can be permanently deleted.")} pending={pending} onClose={() => { if (!pending) setRemoving(null); }} onConfirm={() => { const target = removing; setRemoving(null); void remove(target).catch(error => toast(errorText(error), "error")); }} />}
  </div>;
}

function ClientDetails({ client, cameras }: { client: SentinelClient; cameras: Camera[] }) {
  return <section className="view active sarmg-content-stack"><h2>{t("实例详细信息", "Instance details")}</h2><Table aria-label={t("实例详细信息", "Instance details")}><thead><tr><th>{t("名称", "Name")}</th><th>{t("状态", "Status")}</th><th>{t("版本", "Version")}</th><th>{t("最后在线", "Last online")}</th><th>{t("摄像头", "Cameras")}</th></tr></thead><tbody><tr><th scope="row">{client.name}</th><td>{displayLabel(client.status)}</td><td>{client.client_version ?? "—"}</td><td>{client.last_seen_at === null ? "—" : formatDate(client.last_seen_at)}</td><td>{cameras.length}</td></tr></tbody></Table></section>;
}

function CreateClientDialog({ close, changed, toast }: { close(): void; changed(): Promise<void>; toast(message: string, type?: string): void }) {
  const [name, setName] = useState("");
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<{ requestId?: string } | null>(null);
  const busy = useRef(false);
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy.current || name.trim() === "") return;
    busy.current = true; setPending(true); setFailure(null);
    try {
      await request("/clients", isSentinelClient, { method: "POST", body: JSON.stringify({ name: name.trim() }) });
      await changed();
      close();
      toast(t("客户端实例已创建", "Client instance created"), "success");
    } catch (error) {
      setFailure({ requestId: errorRequestId(error) });
    } finally {
      busy.current = false; setPending(false);
    }
  };
  return <Dialog title={t("新建摄像机实例", "Create camera instance")} description={t("每个实例生成一个永久授权码并只允许绑定一台摄像机；同一客户端可重复添加其他实例授权码。", "Each instance generates one permanent authorization code and binds exactly one camera. The same client can add authorization codes for other instances.")} onClose={() => { if (!busy.current) close(); }}>
    <form className="sentinel-business sarmg-content-stack" onSubmit={event => void submit(event)} aria-busy={pending}>
      {failure && <ErrorState requestId={failure.requestId}>{t("实例未能创建，请重试。", "The instance could not be created. Try again.")}</ErrorState>}
      <label>{t("实例名称", "Instance name")}<InstanceNameField autoFocus value={name} onChange={event => setName(event.target.value)} required /></label>
      <div className="sarmg-actions"><Button type="button" onClick={close} disabled={pending}>{t("取消", "Cancel")}</Button><Button type="submit" disabled={pending || name.trim() === ""}>{pending ? t("创建中…", "Creating…") : t("创建实例", "Create instance")}</Button></div>
    </form>
  </Dialog>;
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

function AuditLogView({ failure, audit, refresh }: { failure: { requestId?: string } | null; audit: AuditRow[] | null; refresh(): void }) {
  return <section className="view active sarmg-content-stack">
    {failure ? <ErrorState requestId={failure.requestId} onRetry={refresh}>{t("审计日志暂不可用。", "Audit logs are temporarily unavailable.")}</ErrorState>
      : audit === null ? <LoadingState>{t("正在加载审计日志…", "Loading audit logs…")}</LoadingState>
        : <section className="management-block sarmg-content-panel"><div className="section-heading"><h2>{t("审计日志", "Audit logs")}</h2></div>
          <div className="audit-list">{audit.length === 0 ? <div className="empty-state">{t("暂无审计日志", "No audit logs yet")}</div> : audit.map((row) => <div key={row.id}><span>{displayLabel(row.action)}</span><small>{formatDate(row.created_at)}</small><code>{displayLabel(row.entity_type)}{row.entity_id === null ? "" : " / " + row.entity_id.slice(0, 8)}</code></div>)}</div>
        </section>}
  </section>;
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
