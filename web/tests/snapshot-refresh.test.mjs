import assert from "node:assert/strict";
import { test } from "node:test";
import { createSnapshotRefresh } from "../src/snapshot-refresh.ts";

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

test("a rejected loader does not let a queued snapshot overlap unfinished work", async () => {
  const slow = deferred(), rejected = deferred();
  const calls = [];
  let loaders = [
    async () => { calls.push("failed"); rejected.resolve(); throw new Error("unavailable"); },
    async () => { calls.push("slow"); await slow.promise; calls.push("finished"); },
  ];
  const refresh = createSnapshotRefresh(() => loaders);
  const first = refresh();
  await rejected.promise;
  // Drain the failure's promise reactions while the second loader remains pending.
  await new Promise(resolve => setImmediate(resolve));
  loaders = [async () => { calls.push("latest filter"); }];
  assert.equal(refresh(), first);
  assert.equal(refresh(), first);
  assert.deepEqual(calls, ["failed", "slow"]);
  slow.resolve();
  await first;
  assert.deepEqual(calls, ["failed", "slow", "finished", "latest filter"]);
});

test("failed snapshots reject and allow a subsequent refresh", async () => {
  let fail = true;
  const refresh = createSnapshotRefresh(() => [() => {
    if (fail) throw new Error("loader failed");
    return Promise.resolve();
  }]);
  await assert.rejects(refresh(), /loader failed/);
  fail = false;
  await refresh();
});

test("a rejection without a reason still rejects the snapshot", async () => {
  const refresh = createSnapshotRefresh(() => [() => Promise.reject()]);
  await assert.rejects(refresh());
});
