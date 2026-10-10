import assert from "node:assert/strict";
import test from "node:test";
import { effectiveStatus, isCameraObservation } from "../src/camera-status.ts";

const observed = "2026-10-10T11:00:00Z";
const expires = "2026-10-10T11:01:00Z";
const now = Date.parse("2026-10-10T11:00:30Z");
const online = { enabled: true, status: "online", device_status: "online", observation_status: "fresh", last_observed_at: observed, observation_expires_at: expires };

test("stale or missing observations cannot produce a current online/offline badge", () => {
  assert.equal(effectiveStatus(online, now), "online");
  for (const status of ["online", "offline", "error"]) {
    assert.equal(effectiveStatus({ ...online, status, observation_status: "stale", observation_expires_at: null }, now), "stale");
    assert.equal(effectiveStatus({ ...online, device_status: status, observation_status: "stale" }, now), "stale");
  }
  assert.equal(effectiveStatus({ ...online, observation_status: "unknown", last_observed_at: null, observation_expires_at: null }, now), "unknown");
  assert.equal(effectiveStatus({ ...online, enabled: false, observation_status: "stale" }, now), "disabled");
  assert.equal(effectiveStatus({ ...online, status: "offline" }, now), "offline");
});

test("an unchanged browser snapshot ages out at its server-issued expiry", () => {
  assert.equal(effectiveStatus(online, Date.parse(expires)), "stale");
  assert.equal(effectiveStatus(online, Date.parse(expires) + 1_000), "stale");
  assert.equal(online.status, "online", "the last known state is preserved");
});

test("camera responses require the current observation contract", () => {
  assert.equal(isCameraObservation(online), true);
  assert.equal(isCameraObservation({ ...online, observation_status: "stale", observation_expires_at: null }), true);
  assert.equal(isCameraObservation({ observation_status: "unknown", last_observed_at: null, observation_expires_at: null }), true);
  for (const value of [{}, { ...online, observation_status: "online" }, { ...online, last_observed_at: null }, { ...online, last_observed_at: "bad" }, { ...online, observation_expires_at: null }, { ...online, observation_expires_at: observed }]) {
    assert.equal(isCameraObservation(value), false);
  }
});
