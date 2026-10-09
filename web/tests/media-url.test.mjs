import assert from "node:assert/strict";
import test from "node:test";
import { isSameOriginMediaUrl, requireSameOriginMediaUrl, whepResourceUrl } from "../src/media-url.ts";

const page = "https://monitor.example/app";

test("media bearer URLs stay on the application origin", () => {
  assert.equal(requireSameOriginMediaUrl("/media-hls/cam/index.m3u8", page),
    "https://monitor.example/media-hls/cam/index.m3u8");
  assert.equal(isSameOriginMediaUrl("https://monitor.example/segment.m4s", page), true);
  for (const url of ["https://elsewhere.example/segment.m4s", "//elsewhere.example/segment.m4s",
    "http://monitor.example/segment.m4s", "https://user:password@monitor.example/segment.m4s"]) {
    assert.equal(isSameOriginMediaUrl(url, page), false);
    assert.throws(() => requireSameOriginMediaUrl(url, page));
  }
});

test("WHEP resource locations preserve the gateway and reject foreign origins", () => {
  const request = "https://monitor.example/media-webrtc/cam/whep";
  assert.equal(whepResourceUrl("/cam/whep/session", request, page),
    "https://monitor.example/media-webrtc/cam/whep/session");
  assert.equal(whepResourceUrl("https://monitor.example/media-webrtc/cam/whep/session", request, page),
    "https://monitor.example/media-webrtc/cam/whep/session");
  assert.equal(whepResourceUrl(null, request, page), null);
  assert.throws(() => whepResourceUrl("https://elsewhere.example/cam/whep/session", request, page));
});
