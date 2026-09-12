import { checkWebLanguage } from "./language.mjs";
import assert from "node:assert/strict";
import { chromium, firefox, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { preview } from "vite";

const time = "2026-09-04T00:00:00Z";
const administratorId = "A".repeat(43);
const session = { authenticated: true, user_id: administratorId, username: "admin", role: "admin", csrf_token: "A".repeat(43) };
const camera = { id: "018f1f4b-7a5d-7b5f-8d31-123456789abc", name: "验收摄像头", location: "测试现场", has_sub_stream: false, onvif_configured: true, username: null, enabled: false, record_enabled: false, status: "disabled", last_seen_at: null, created_at: time, updated_at: time };
const server = await preview({ preview: { host: "127.0.0.1", port: 0, strictPort: true } });
const address = server.httpServer.address();
assert.ok(address && typeof address === "object");
try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const context = await browser.newContext({ locale: "zh-CN",  viewport: { width: 360, height: 740 } });
      const page = await context.newPage();
      const errors = [], paths = [], ptz = [];
      let cameraDeleted = false, acknowledged = false, failStatus = false;
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/api/v2/**", async route => {
        const request = route.request(), url = new URL(request.url()), path = url.pathname;
        paths.push(path);
        if (path.endsWith("/auth/session")) return route.fulfill({ json: session });
        if (path.endsWith("/events/stream")) return route.fulfill({ status: 200, contentType: "text/event-stream", body: ": acceptance\n\n" });
        if (request.method() !== "GET") assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
        if (path.endsWith("/ptz")) { ptz.push(request.postDataJSON().action); return route.fulfill({ status: 204 }); }
        if (path.endsWith("/cameras") && request.method() === "GET") return route.fulfill({ json: cameraDeleted ? [] : [camera] });
        if (path.endsWith(`/cameras/${camera.id}`) && request.method() === "DELETE") {
          cameraDeleted = true;
          return route.fulfill({ json: { id: "operation-1", camera_id: camera.id, generation: 1, kind: "delete", state: "Queued", reason: "administrator", requested_by: administratorId, attempt: 0, max_attempts: 1, created_at: time, started_at: null, finished_at: null, retry_at: null, error_code: null, error_message: null } });
        }
        if (path.endsWith("/events/event-1/ack")) { acknowledged = true; return route.fulfill({ status: 204 }); }
        if (path.endsWith("/events")) return route.fulfill({ json: [{ id: "event-1", camera_id: camera.id, kind: "camera.status", severity: "info", message: "验收事件", acknowledged_at: acknowledged ? time : null, created_at: time }] });
        if (path.endsWith("/system/status")) {
          if (failStatus) { failStatus = false; return route.fulfill({ status: 500, json: { code: "platform.internal", message: "SECRET database path", retryable: false, request_id: "system-failure-123" } }); }
          return route.fulfill({ json: { service: "sentinel-monitor", version: "0.2.4", database: "ok", media_service: "ok", cameras: { total: 1, online: 0, recording: 0 }, server_time: time } });
        }
        if (path.endsWith("/audit")) return route.fulfill({ json: [{ id: "audit-1", action: "camera.updated", entity_type: "camera", entity_id: camera.id, created_at: time }] });
        if (path.endsWith("/recordings")) { assert.equal(url.searchParams.get("camera_id"), camera.id); return route.fulfill({ json: [{ start: time, duration: 60 }] }); }
        throw new Error(`Unexpected API request ${request.method()} ${path}`);
      });
      await page.goto(`http://127.0.0.1:${address.port}/#cameras`);
      await expect(page.getByRole("complementary").getByText(camera.name, { exact: true })).toBeVisible();
      await page.getByRole("button", { name: "新建摄像头", exact: true }).click();
      const editor = page.getByRole("dialog", { name: "添加摄像头", exact: true });
      await expect(editor).toBeVisible();
      for (let i = 0; i < 12; i++) { await page.keyboard.press("Tab"); assert.ok(await editor.evaluate(element => element.contains(document.activeElement))); }
      await page.keyboard.press("Escape");
      await expect(page.getByRole("button", { name: "新建摄像头", exact: true })).toBeFocused();
      await page.getByRole("button", { name: "主码流", exact: true }).click();
      const movement = page.getByRole("button", { name: "云台向上", exact: true });
      await movement.focus(); await page.keyboard.down("Space"); await page.keyboard.up("Space");
      await expect.poll(() => ptz.slice()).toEqual(["move", "stop"]);
      await movement.focus(); await page.keyboard.down("Enter");
      await page.evaluate(() => window.dispatchEvent(new Event("blur")));
      await page.keyboard.up("Enter");
      await expect.poll(() => ptz.slice()).toEqual(["move", "stop", "move", "stop"]);
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: "录像检索", exact: true }).click();
      await page.getByRole("button", { name: "查询录像", exact: true }).click();
      await expect(page.getByRole("button", { name: /1分0秒/ })).toBeVisible();
      await page.getByRole("button", { name: "事件中心", exact: true }).click();
      const filter = page.getByRole("checkbox", { name: "仅显示未确认事件", exact: true });
      const filterBox = await filter.boundingBox();
      assert.ok(filterBox.width <= 24 && filterBox.height <= 24);
      await filter.check(); await expect(filter).toBeChecked();
      await filter.uncheck();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByRole("cell", { name: "已确认", exact: true })).toBeVisible();
      await page.getByRole("button", { name: "系统管理", exact: true }).click();
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.getByRole("banner").locator('.sarmg-product-identity')).toHaveText("Sentinel Monitor");
      await expect(page.getByText("运行正常", { exact: true })).toBeVisible();
      await expect(page.getByText("更新摄像头", { exact: true })).toBeVisible();
      const statusTable = page.getByRole("table", { name: "系统状态", exact: true });
      await expect(statusTable.getByRole("rowheader")).toHaveText(["媒体服务", "在线设备", "录像任务"]);
      await expect(statusTable.getByRole("columnheader")).toHaveText(["项目", "当前状态", "说明"]);
      failStatus = true;
      await page.getByRole("group", { name: "全局操作" }).getByRole("button", { name: "刷新", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText("system-failure-123");
      await expect(page.locator("body")).not.toContainText("SECRET");
      await expect(page.getByText("运行正常", { exact: true })).toHaveCount(0);
      await page.getByRole("alert").getByRole("button", { name: "重试" }).click();
      await expect(page.getByText("运行正常", { exact: true })).toBeVisible();
      await expect(page.getByRole("heading", { name: "管理员账号", exact: true })).toHaveCount(0);
      await expect(page.getByRole("table", { name: "管理员账号", exact: true })).toHaveCount(0);

      for (const theme of ["light", "dark"]) {
        if (await page.locator("html").getAttribute("data-theme") !== theme) await page.getByRole("button", { name: /切换到.*模式/ }).click();
        assert.deepEqual((await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze()).violations, []);
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      }
      await page.getByRole("button", { name: "实时监控", exact: true }).click();
      await page.getByRole("button", { name: "删除", exact: true }).click();
      await expect(page.getByRole("button", { name: "取消", exact: true })).toBeFocused();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByText("还没有匹配的摄像头。", { exact: true })).toBeVisible();
      assert.ok(!paths.some(path => path.includes("/users")));
      await checkWebLanguage(page, {"routes":[["cameras","Live monitoring"],["recordings","Recordings"],["events","Events"],["system","System management"]],"names":["验收摄像头","测试现场"]});
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: current Sentinel system/cameras/recordings/events, PTZ stop, account settings, modal focus and mobile WCAG AA passed`);
      await context.close();
    } finally { await browser.close(); }
  }
} finally { await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve())); }
