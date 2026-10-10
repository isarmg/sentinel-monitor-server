import assert from "node:assert/strict";
import test from "node:test";
import { loadOriginalMain } from "./fixtures/original-main.mjs";
import { HookHost, walk, textContent } from "./fixtures/hook-host.mjs";

const { RecordingsView, CameraDrawer, localDateInput } = await loadOriginalMain();
const settle = () => new Promise(resolve => setImmediate(resolve));
function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
const segment = { start: "2026-10-01T12:00:00Z", duration: 60 };
function recordings(t) {
  const host = new HookHost(RecordingsView, {
    cameras: [{ id: "camera-a", name: "A" }, { id: "camera-b", name: "B" }],
    toast() {},
  });
  t.after(() => host.unmount());
  const nodes = predicate => walk(host.render(), predicate);
  const date = (index, value) => nodes(node => node.type === "input")[index].props.onChange({ target: { value } });
  const range = (day = "01") => { date(0, `2026-10-${day}T00:00`); date(1, `2026-10-${day}T23:59`); };
  const search = () => nodes(node => node.type === "button" && textContent(node) === "Search recordings")[0].props.onClick();
  const rows = () => nodes(node => node.props.className === "record-item");
  const video = () => nodes(node => node.type === "video")[0];
  range();
  return { host, nodes, date, range, search, rows, video };
}

test("recordings distinguish initial, loading, failed, retry, and empty results", async t => {
  const { host, nodes, range, search, rows, video } = recordings(t);
  assert.match(textContent(host.render()), /Choose a time range and search/);
  assert.doesNotMatch(textContent(host.render()), /No recordings in the selected range/);
  globalThis.probeRequest = async () => [segment];
  search();
  assert.equal(nodes(node => node.type === "loading").length, 1);
  await settle();
  rows()[0].props.onClick();
  assert.match(video().props.src, /2026-10-01/);
  const playingKey = video().key;
  assert.equal(playingKey, video().props.src);
  const refresh = deferred();
  globalThis.probeRequest = () => refresh.promise;
  search();
  assert.equal(rows().length, 0, "a repeated query clears the previous result immediately");
  assert.equal(video().props.src, undefined);
  assert.notEqual(video().key, playingKey, "clearing playback replaces the media element, not just its src attribute");
  refresh.resolve([segment]);
  await settle();
  rows()[0].props.onClick();
  assert.equal(video().key, playingKey, "selecting the same recording after reset mounts its media element again");
  range("02");
  assert.notEqual(video().key, playingKey, "editing dates unmounts the active recording");
  const failed = deferred();
  globalThis.probeRequest = () => failed.promise;
  search();
  assert.equal(rows().length, 0);
  assert.equal(video().props.src, undefined);
  failed.reject(new Error("Temporary request failure"));
  await settle();
  assert.equal(nodes(node => node.type === "loading").length, 0);
  assert.match(textContent(nodes(node => node.type === "error")[0]), /could not be loaded/);
  assert.equal(rows().length, 0);
  globalThis.probeRequest = async () => [];
  search();
  await settle();
  assert.equal(nodes(node => node.type === "error").length, 0);
  assert.match(textContent(host.render()), /No recordings in the selected range/);
});

test("invalid or reversed recording dates fail locally and allow a retry", async t => {
  const { nodes, date, range, search, rows } = recordings(t);
  let calls = 0;
  globalThis.probeRequest = async () => { calls += 1; return [segment]; };
  for (const start of ["", "not-a-date", "2026-10-03T00:00"]) {
    date(0, start);
    search();
    await settle();
    assert.equal(nodes(node => node.type === "loading").length, 0);
    assert.equal(nodes(node => node.type === "error").length, 1);
    assert.equal(calls, 0);
  }
  range();
  search();
  await settle();
  assert.equal(calls, 1);
  assert.equal(rows().length, 1);
});

test("recording date edits and newer searches ignore old success and failure", async t => {
  const { host, nodes, range, search, rows } = recordings(t);
  const old = deferred(), latest = deferred();
  globalThis.probeRequest = () => old.promise;
  search();
  range("02");
  host.render();
  old.resolve([segment]);
  await settle();
  assert.equal(rows().length, 0);
  assert.match(textContent(host.render()), /Choose a time range and search/);
  const superseded = deferred();
  globalThis.probeRequest = () => superseded.promise;
  search();
  globalThis.probeRequest = () => latest.promise;
  search();
  latest.resolve([{ ...segment, start: "2026-10-02T12:00:00Z" }]);
  await settle();
  superseded.reject(new Error("Old request failure"));
  await settle();
  assert.equal(nodes(node => node.type === "error").length, 0);
  assert.equal(rows().length, 1);
  assert.match(textContent(rows()[0]), /10\/02\/2026/);
});

test("switching recording camera clears playback and ignores its pending request", async t => {
  const { host, nodes, search, rows, video } = recordings(t);
  globalThis.probeRequest = async () => [segment];
  search();
  await settle();
  rows()[0].props.onClick();
  assert.match(video().props.src, /camera-a/);
  const old = deferred();
  globalThis.probeRequest = () => old.promise;
  search();
  nodes(node => node.type === "select")[0].props.onChange({ target: { value: "camera-b" } });
  host.render();
  old.resolve([segment]);
  await settle();
  assert.equal(rows().length, 0);
  assert.equal(video().props.src, undefined);
  assert.match(textContent(host.render()), /Choose a time range and search/);
});

test("recording local date inputs round-trip ordinary winter and summer instants", () => {
  for (const iso of ["2026-01-15T12:34:00.000Z", "2026-07-15T12:34:00.000Z"]) {
    assert.equal(new Date(localDateInput(new Date(iso))).toISOString(), iso);
  }
});

test("retrying a failed PTZ status read only repeats GET, never its physical command", async t => {
  const timers = new Map(), requests = [];
  let nextTimer = 0, fail = true, successfulReads = 0;
  t.mock.method(window, "setTimeout", fn => { timers.set(++nextTimer, fn); return nextTimer; });
  t.mock.method(window, "clearTimeout", id => timers.delete(id));
  const camera = { id: "camera-a", name: "A", location: "", adapter_kind: "onvif", capabilities: { ptz: "supported" } };
  const host = new HookHost(CameraDrawer, { camera, close() {}, toast() {} });
  t.after(() => host.unmount());
  const find = predicate => walk(host.render(), predicate)[0];
  const poll = async () => {
    assert.equal(timers.size, 1);
    const [id, fn] = [...timers.entries()][0];
    timers.delete(id); fn(); await settle(); host.render();
  };
  globalThis.probeRequest = async (path, _guard, options) => {
    requests.push({ path, method: options?.method ?? "GET" });
    if (options?.method === "POST") return { command_id: "stop-1", state: "queued" };
    if (fail) throw new Error("Transient read failure");
    successfulReads += 1;
    return { command_id: "stop-1", state: successfulReads === 1 ? "awaiting_result" : "succeeded" };
  };
  find(node => node.props["aria-label"] === "Stop movement").props.onClick();
  await settle(); host.render();
  await poll();
  assert.match(textContent(find(node => node.type === "error")), /could not be checked/);
  assert.equal(find(node => node.props.role === "status"), undefined);
  assert.equal(timers.size, 0);
  fail = false;
  find(node => node.type === "button" && textContent(node) === "Retry status lookup").props.onClick();
  host.render(); await poll();
  assert.equal(textContent(find(node => node.props.role === "status")), "Sent; waiting for the device result");
  await poll();
  assert.equal(find(node => node.type === "error"), undefined);
  assert.equal(textContent(find(node => node.props.role === "status")), "The device confirmed execution");
  assert.equal(timers.size, 0);
  assert.deepEqual(requests.map(request => request.method), ["POST", "GET", "GET", "GET"]);
  assert.equal(requests[1].path, requests[2].path);
});


test("PTZ controls follow supported, unsupported, and unknown capabilities", () => {
  for (const support of ["supported", "unsupported", "unknown"]) {
    const camera = { id: "camera-a", name: "A", location: "", adapter_kind: "onvif", capabilities: { ptz: support } };
    const host = new HookHost(CameraDrawer, { camera, close() {}, toast() {} });
    const panel = walk(host.render(), node => node.props["aria-label"] === "PTZ controls");
    assert.equal(panel.length, support === "supported" ? 1 : 0);
    host.unmount();
  }
});
