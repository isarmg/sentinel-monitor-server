import { checkWebLanguage } from "./language.mjs";
import assert from "node:assert/strict";
import { chromium, firefox, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { preview } from "vite";

const time = "2026-09-04T00:00:00Z";
const administratorId = "A".repeat(43);
const session = { authenticated: true, user_id: administratorId, username: "admin", role: "admin", csrf_token: "A".repeat(43) };
const camera = { id: "018f1f4b-7a5d-7b5f-8d31-123456789abc", name: "验收摄像头", location: "测试现场", has_sub_stream: false, onvif_configured: true, username: null, source_kind: "direct", client_id: null, adapter_kind: "server_direct", manufacturer: null, model: null, firmware_version: null, serial_number: null, capabilities: { video: true, main_stream: true, sub_stream: false, local_recording: false, server_recording: true, ptz: true, events: false, audio_input: false, audio_output: false }, streams: [{ profile: "main", video_codec: null, audio_codec: null, width: null, height: null, frame_rate: null }], health_message: null, device_status: "disabled", storage_mode: "server", enabled: false, record_enabled: false, status: "disabled", last_seen_at: null, created_at: time, updated_at: time };
const clientInstance = { id: "018f1f4b-7a5d-7b5f-8d31-123456789abd", installation_id: null, name: "验收客户端", client_version: null, authorization_code: "s".repeat(64), status: "pending", last_seen_at: null, created_at: time, updated_at: time };
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
      let cameraDeleted = false, acknowledged = false, failStatus = false, clients = [{ ...clientInstance }];
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/api/v2/**", async route => {
        const request = route.request(), url = new URL(request.url()), path = url.pathname;
        paths.push(path);
        if (path.endsWith("/auth/session")) return route.fulfill({ json: session });
        if (path.endsWith("/events/stream")) return route.fulfill({ status: 200, contentType: "text/event-stream", body: ": acceptance\n\n" });
        if (request.method() !== "GET") assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
        if (path.endsWith("/clients") && request.method() === "GET") return route.fulfill({ json: clients });
        if (path.endsWith(`/clients/${clientInstance.id}`) && request.method() === "DELETE") {
          if (clients[0]?.status === "revoked") clients = []; else clients[0].status = "revoked";
          return route.fulfill({ status: 204 });
        }
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
          return route.fulfill({ json: { service: "sentinel-monitor", version: "0.2.7", database: "ok", media_service: "ok", cameras: { total: 1, online: 0, recording: 0 }, server_time: time } });
        }
        if (path.endsWith("/audit")) return route.fulfill({ json: [{ id: "audit-1", action: "camera.updated", entity_type: "camera", entity_id: camera.id, created_at: time }] });
        if (path.endsWith("/recordings")) { assert.equal(url.searchParams.get("camera_id"), camera.id); return route.fulfill({ json: [{ start: time, duration: 60 }] }); }
        throw new Error(`Unexpected API request ${request.method()} ${path}`);
      });
      await page.goto(`http://127.0.0.1:${address.port}/#instances`);
      await expect(page.getByRole("table", { name: "摄像头实例列表" }).getByRole("button", { name: camera.name, exact: true })).toBeVisible();
      await expect(page.getByRole("complementary")).toHaveCount(0);
      const menuToFirst = await page.evaluate(() => {
        const header = document.querySelector(".sarmg-page-header");
        const first = document.querySelector(".sarmg-instance-workspace");
        if (!header || !first) throw new Error("Sentinel spacing fixture is incomplete");
        return first.getBoundingClientRect().top - header.getBoundingClientRect().bottom;
      });
      assert.ok(Math.abs(menuToFirst - 16) < 2, String(menuToFirst));
      await page.getByRole("button", { name: "取消配对", exact: true }).click();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByRole("cell", { name: "已撤销", exact: true })).toBeVisible();
      await page.getByRole("button", { name: "删除实例", exact: true }).click();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByText("尚无客户端实例", { exact: true })).toBeVisible();
      await page.getByRole("button", { name: "新建直连摄像头", exact: true }).click();
      const editor = page.getByRole("dialog", { name: "添加摄像头", exact: true });
      await expect(editor).toBeVisible();
      for (let i = 0; i < 12; i++) { await page.keyboard.press("Tab"); assert.ok(await editor.evaluate(element => element.contains(document.activeElement))); }
      await page.keyboard.press("Escape");
      await expect(page.getByRole("button", { name: "新建直连摄像头", exact: true })).toBeFocused();
      await page.getByRole("table", { name: "摄像头实例列表" }).getByRole("button", { name: camera.name, exact: true }).click();
      await expect(page.getByRole("button", { name: "详细信息", exact: true })).toHaveAttribute("aria-pressed", "true");
      await page.getByRole("button", { name: "主码流", exact: true }).click();
      const movement = page.getByRole("button", { name: "云台向上", exact: true });
      await movement.focus(); await page.keyboard.down("Space"); await page.keyboard.up("Space");
      await expect.poll(() => ptz.slice()).toEqual(["move", "stop"]);
      await movement.focus(); await page.keyboard.down("Enter");
      await page.evaluate(() => window.dispatchEvent(new Event("blur")));
      await page.keyboard.up("Enter");
      await expect.poll(() => ptz.slice()).toEqual(["move", "stop", "move", "stop"]);
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: "日志", exact: true }).click();
      await page.getByRole("button", { name: "查询录像", exact: true }).click();
      await expect(page.getByRole("button", { name: /1分0秒/ })).toBeVisible();
      const filter = page.getByRole("checkbox", { name: "仅显示未确认事件", exact: true });
      const filterBox = await filter.boundingBox();
      assert.ok(filterBox.width <= 24 && filterBox.height <= 24);
      await filter.check(); await expect(filter).toBeChecked();
      await filter.uncheck();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByRole("cell", { name: "已确认", exact: true })).toBeVisible();
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.getByRole("banner").locator('.sarmg-product-identity')).toHaveText("Sentinel Monitor");
      await expect(page.getByText("运行正常", { exact: true })).toBeVisible();
      await expect(page.getByText("更新摄像头", { exact: true })).toBeVisible();
      const statusTable = page.getByRole("table", { name: "系统状态", exact: true });
      await expect(statusTable.getByRole("rowheader")).toHaveText(["媒体服务", "在线设备", "录像任务"]);
      await expect(statusTable.getByRole("columnheader")).toHaveText(["项目", "当前状态", "说明"]);
      const sectionGap = await page.evaluate(() => {
        const table = document.querySelector('table[aria-label="系统状态"]')?.closest(".sarmg-table-scroll");
        const section = document.querySelector(".management-block");
        if (!table || !section) throw new Error("Sentinel section spacing fixture is incomplete");
        return section.getBoundingClientRect().top - table.getBoundingClientRect().bottom;
      });
      assert.ok(Math.abs(sectionGap - 16) < 2, String(sectionGap));
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
      await page.getByRole("button", { name: "详细信息", exact: true }).click();
      await page.getByRole("button", { name: "删除", exact: true }).click();
      await expect(page.getByRole("button", { name: "取消", exact: true })).toBeFocused();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByText("还没有匹配的摄像头。", { exact: true })).toBeVisible();
      assert.ok(!paths.some(path => path.includes("/users")));
      await checkWebLanguage(page, {"routes":[["instances","Instance list"],["logs","Logs"]],"names":["验收摄像头","测试现场"]});
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: current Sentinel system/cameras/recordings/events, PTZ stop, account settings, modal focus and mobile WCAG AA passed`);
      await context.close();
    } finally { await browser.close(); }
  }
} finally { await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve())); }
