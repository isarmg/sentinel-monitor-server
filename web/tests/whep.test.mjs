import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { registerHooks } from "node:module";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { transformWithOxc } from "vite";

// Execute the original player with in-memory browser boundaries, not a browser.
const entry = new URL("../src/whep.ts", import.meta.url);
const { code } = await transformWithOxc(await readFile(entry, "utf8"), fileURLToPath(entry));
const hooks = registerHooks({
  resolve(specifier, context, next) {
    if (context.parentURL === entry.href && specifier === "./media-url") {
      return { url: new URL("../src/media-url.ts", import.meta.url).href, shortCircuit: true };
    }
    return next(specifier, context);
  },
  load(url, context, next) {
    if (url === entry.href) return { format: "module", source: code, shortCircuit: true };
    return next(url, context);
  },
});
const { WhepPlayer } = await import(entry.href).finally(() => hooks.deregister());

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}
const settle = () => new Promise(resolve => setImmediate(resolve));

function browserFixture(t, plans = []) {
  const peers = [], calls = [];
  class Peer extends EventTarget {
    constructor() {
      super();
      this.plan = plans[peers.length] ?? {};
      this.stream = { id: `stream-${peers.length + 1}` };
      this.connectionState = "new";
      this.iceGatheringState = "complete";
      this.closeCount = 0;
      peers.push(this);
    }
    addTransceiver() {}
    async createOffer() {
      if (this.plan.offer) await this.plan.offer.promise;
      return { type: "offer", sdp: "test offer" };
    }
    async setLocalDescription(offer) { this.localDescription = offer; }
    async setRemoteDescription() {
      this.connectionState = "connected";
      this.ontrack?.({ streams: [this.stream] });
    }
    close() { this.closeCount += 1; this.connectionState = "closed"; }
  }
  for (const [name, value] of Object.entries({
    window: { location: { href: "https://camera.example/" }, setTimeout, clearTimeout },
    RTCPeerConnection: Peer,
  })) {
    const previous = Object.getOwnPropertyDescriptor(globalThis, name);
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });
    t.after(() => {
      if (previous) Object.defineProperty(globalThis, name, previous);
      else delete globalThis[name];
    });
  }
  t.mock.method(globalThis, "fetch", async (url, options = {}) => {
    calls.push({ url, method: options.method });
    if (options.method === "POST") {
      const peer = peers.at(-1);
      if (peer.plan.post) await peer.plan.post.promise;
      return new Response("test answer", { headers: { Location: `${url}/session` } });
    }
    return new Response(null, { status: 204 });
  });
  const video = { srcObject: null, play: async () => {} };
  const player = (path = "main") => {
    const value = new WhepPlayer(video, `/${path}/whep`, "test-token");
    t.after(() => value.close());
    return value;
  };
  return { video, peers, calls, player };
}

test("close detaches its own stream and deletes its resource only once", async t => {
  const { video, peers, calls, player } = browserFixture(t);
  const current = player();
  await current.start();
  assert.equal(video.srcObject, peers[0].stream);
  current.close();
  current.close();
  assert.equal(video.srcObject, null);
  assert.equal(peers[0].closeCount, 1);
  assert.deepEqual(calls.filter(call => call.method === "DELETE"), [
    { url: "https://camera.example/main/whep/session", method: "DELETE" },
  ]);
});

test("closing an older player preserves a replacement stream", async t => {
  const { video, peers, player } = browserFixture(t);
  const old = player("main"), current = player("sub");
  await old.start();
  await current.start();
  old.close();
  assert.equal(video.srcObject, peers[1].stream);
  assert.equal(peers[1].connectionState, "connected");
});

test("an interrupted offer cannot clear the next player's connected video", async t => {
  const offer = deferred();
  const { video, peers, player } = browserFixture(t, [{ offer }]);
  const old = player("main");
  const pending = old.start();
  const rejected = assert.rejects(pending, { name: "AbortError" });
  await settle();
  old.close();
  await player("sub").start();
  offer.resolve();
  await rejected;
  assert.equal(video.srcObject, peers[1].stream);
  assert.equal(peers[1].connectionState, "connected");
});

test("a late resource is still deleted without clearing the replacement video", async t => {
  const post = deferred();
  const { video, peers, calls, player } = browserFixture(t, [{ post }]);
  const old = player("main");
  const rejected = assert.rejects(old.start(), { name: "AbortError" });
  await settle();
  assert.equal(calls.at(-1).method, "POST");
  old.close();
  await player("sub").start();
  post.resolve();
  await rejected;
  old.close();
  assert.equal(video.srcObject, peers[1].stream);
  assert.deepEqual(calls.filter(call => call.method === "DELETE"), [
    { url: "https://camera.example/main/whep/session", method: "DELETE" },
  ]);
});

test("a closed player's late track callback cannot replace the current stream", async t => {
  const { video, peers, player } = browserFixture(t);
  const old = player("main");
  await old.start();
  const oldTrack = peers[0].ontrack;
  old.close();
  await player("sub").start();
  oldTrack({ streams: [peers[0].stream] });
  assert.equal(video.srcObject, peers[1].stream);
});
