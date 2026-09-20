import { t } from "@sarmg/admin-ui/i18n";
function aborted(): DOMException {
  return new DOMException("Player closed", "AbortError");
}

function waitForIceGathering(peer: RTCPeerConnection, signal: AbortSignal, timeoutMs = 5_000): Promise<void> {
  if (peer.iceGatheringState === "complete") return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timeout = window.setTimeout(() => done(), timeoutMs);
    function done(error?: DOMException) {
      window.clearTimeout(timeout);
      peer.removeEventListener("icegatheringstatechange", changed);
      signal.removeEventListener("abort", cancelled);
      if (error === undefined) resolve(); else reject(error);
    }
    function changed() {
      if (peer.iceGatheringState === "complete") done();
    }
    function cancelled() { done(aborted()); }
    peer.addEventListener("icegatheringstatechange", changed);
    signal.addEventListener("abort", cancelled, { once: true });
    if (signal.aborted) cancelled();
  });
}

function waitForConnection(peer: RTCPeerConnection, signal: AbortSignal, timeoutMs = 12_000): Promise<void> {
  return new Promise((resolve, reject) => {
    const timeout = window.setTimeout(
      () => done(new Error(t("WebRTC连接超时", "WebRTC connection timed out"))),
      timeoutMs,
    );
    function done(error?: Error | DOMException) {
      window.clearTimeout(timeout);
      peer.removeEventListener("connectionstatechange", changed);
      signal.removeEventListener("abort", cancelled);
      if (error === undefined) resolve(); else reject(error);
    }
    function changed() {
      if (peer.connectionState === "connected") done();
      else if (["failed", "closed"].includes(peer.connectionState)) {
        done(new Error(t("WebRTC连接失败", "WebRTC connection failed")));
      }
    }
    function cancelled() { done(aborted()); }
    peer.addEventListener("connectionstatechange", changed);
    signal.addEventListener("abort", cancelled, { once: true });
    if (signal.aborted) cancelled(); else changed();
  });
}

function parseIceServers(header: string | null): RTCIceServer[] {
  if (header === null) return [];
  return header
    .split(/,(?=\s*<)/)
    .flatMap((entry): RTCIceServer[] => {
      const url = entry.match(/<([^>]+)>/)?.[1];
      if (url === undefined || !/rel="?ice-server"?/i.test(entry)) return [];
      const username = entry.match(/username="([^"]*)"/i)?.[1];
      const credential = entry.match(/credential="([^"]*)"/i)?.[1];
      return [{
        urls: [url],
        ...(username === undefined ? {} : { username }),
        ...(credential === undefined ? {} : { credential }),
      }];
    });
}

function resourceUrl(location: string | null, requestUrl: string): string | null {
  if (location === null || location === "") return null;
  if (/^https?:\/\//i.test(location)) return location;
  if (location.startsWith("/") && requestUrl.startsWith("/media-webrtc/")) {
    return `/media-webrtc${location}`;
  }
  return new URL(location, new URL(requestUrl, window.location.href)).toString();
}

export class WhepPlayer {
  private peer: RTCPeerConnection | null = null;
  private resource: string | null = null;
  private readonly controller = new AbortController();
  private closed = false;

  constructor(
    private readonly video: HTMLVideoElement,
    private readonly url: string,
    private readonly token: string,
    private readonly connectionLost?: () => void,
  ) {}

  async start(): Promise<void> {
    try {
      const headers = { Authorization: `Bearer ${this.token}` };
      let iceServers: RTCIceServer[] = [];
      try {
        const options = await fetch(this.url, { method: "OPTIONS", headers, signal: this.controller.signal });
        iceServers = parseIceServers(options.headers.get("Link"));
      } catch {
        if (this.closed) throw aborted();
        // LAN deployment can still connect by using candidates in the SDP.
      }

      const peer = new RTCPeerConnection({ iceServers });
      if (this.closed) { peer.close(); throw aborted(); }
      this.peer = peer;
      peer.addTransceiver("video", { direction: "recvonly" });
      peer.addTransceiver("audio", { direction: "recvonly" });
      peer.ontrack = (event) => {
        const stream = event.streams[0];
        if (stream !== undefined) this.video.srcObject = stream;
      };

      const offer = await peer.createOffer();
      if (this.closed) throw aborted();
      await peer.setLocalDescription(offer);
      await waitForIceGathering(peer, this.controller.signal);
      const localDescription = peer.localDescription;
      if (localDescription === null) throw new Error(t("浏览器未生成WHEP SDP", "The browser did not generate a WHEP SDP"));
      const response = await fetch(this.url, {
        method: "POST",
        headers: { ...headers, "Content-Type": "application/sdp" },
        body: localDescription.sdp,
        signal: this.controller.signal,
      });
      if (!response.ok) throw new Error(t("WHEP信令失败 ({0})", "WHEP signaling failed ({0})", [response.status]));
      this.resource = resourceUrl(response.headers.get("Location"), this.url);
      if (this.closed) throw aborted();
      await peer.setRemoteDescription({ type: "answer", sdp: await response.text() });
      await waitForConnection(peer, this.controller.signal);
      if (this.closed) throw aborted();
      peer.onconnectionstatechange = () => {
        if (peer.connectionState === "failed" && !this.closed) {
          this.connectionLost?.();
        }
      };
      await this.video.play().catch(() => undefined);
    } catch (error) {
      this.close();
      throw error;
    }
  }

  close(): void {
    this.closed = true;
    this.controller.abort();
    this.deleteResource();
    if (this.peer !== null) {
      this.peer.onconnectionstatechange = null;
      this.peer.ontrack = null;
      this.peer.close();
    }
    this.peer = null;
    this.video.srcObject = null;
  }

  private deleteResource(): void {
    const resource = this.resource;
    this.resource = null;
    if (resource !== null) {
      void fetch(resource, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${this.token}` },
        keepalive: true,
      }).catch(() => undefined);
    }
  }
}
