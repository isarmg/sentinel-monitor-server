import { checkWebLanguage } from "./language.mjs";
import assert from "node:assert/strict";
import { chromium, firefox, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { preview } from "vite";

const time = "2026-09-04T00:00:00Z";
async function assertColumnContentAlignment(table) {
  const offsets = await table.evaluate(element => {
    const textStart = cell => {
      const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT); let text;
      while ((text = walker.nextNode()) && !text.textContent.trim()) {}
      if (!text) throw new Error("table cell has no visible text");
      const range = document.createRange(); range.selectNodeContents(text);
      return range.getBoundingClientRect().left;
    };
    const contentStart = cell => textStart(cell);
    const headings = [...element.querySelectorAll("thead th")], values = [...element.querySelector("tbody tr").children];
    if (headings.length !== values.length) throw new Error("table column count mismatch");
    return headings.map((heading, index) => Math.abs(textStart(heading) - contentStart(values[index])));
  });
  assert.ok(offsets.every(offset => offset < 0.5), `column content offsets: ${JSON.stringify(offsets)}`);
}
const administratorId = "A".repeat(43);
const session = { authenticated: true, user_id: administratorId, username: "admin", role: "admin", csrf_token: "A".repeat(43) };
const activeId = "018f1f4b-7a5d-7b5f-8d31-123456789abc";
const pendingId = "018f1f4b-7a5d-7b5f-8d31-123456789abd";
const camera = { id: activeId, name: "验收摄像头", location: "测试现场", has_sub_stream: false, source_kind: "client", client_id: activeId, adapter_kind: "onvif", manufacturer: "Acme", model: "IPC-1", firmware_version: null, serial_number: null, capabilities: { video: true, main_stream: true, sub_stream: false, local_recording: false, server_recording: true, ptz: true, events: false, audio_input: false, audio_output: false }, streams: [{ profile: "main", video_codec: null, audio_codec: null, width: null, height: null, frame_rate: null }], health_message: null, device_status: "disabled", storage_mode: "server", enabled: false, record_enabled: false, status: "disabled", last_seen_at: null, created_at: time, updated_at: time };
const activeInstance = { id: activeId, installation_id: "018f1f4b-7a5d-7b5f-8d31-123456789abe", name: "门口摄像机实例", client_version: "0.3.0", authorization_code: "a".repeat(36), status: "online", last_seen_at: time, created_at: time, updated_at: time };
const pendingInstance = { id: pendingId, installation_id: null, name: "待配对摄像机", client_version: null, authorization_code: "s".repeat(36), status: "pending", last_seen_at: null, created_at: time, updated_at: time };
const server = await preview({ preview: { host: "127.0.0.1", port: 0, strictPort: true } });
const address = server.httpServer.address();
assert.ok(address && typeof address === "object");
try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const context = await browser.newContext({ locale: "zh-CN",  viewport: { width: 360, height: 740 } });
      const page = await context.newPage();
      const errors = [], paths = [], ptz = [], eventQueries = [];
      let acknowledged = false, failAudit = false, holdSystem = false, releaseSystem = null, clients = [{ ...activeInstance }, { ...pendingInstance }];
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/api/v2/**", async route => {
        const request = route.request(), url = new URL(request.url()), path = url.pathname;
        paths.push(path);
        if (path.endsWith("/auth/session")) return route.fulfill({ json: session });
        if (path.endsWith("/events/stream")) return route.fulfill({ status: 200, contentType: "text/event-stream", body: ": acceptance\n\n" });
        if (path.endsWith("/system/status")) {
          if (holdSystem) { holdSystem = false; await new Promise(resolve => { releaseSystem = resolve; }); releaseSystem = null; }
          return route.fulfill({ json: { service: "sentinel-monitor", version: "0.2.19", database: "ok", media_service: "ok", cameras: { recording_configured: 0 }, server_time: time } });
        }
        if (request.method() !== "GET") assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
        if (path.endsWith("/clients") && request.method() === "GET") return route.fulfill({ json: clients });
        if (path.endsWith("/clients") && request.method() === "POST") {
          assert.deepEqual(request.postDataJSON(), { name: "新实例" });
          const created = { ...pendingInstance, id: "018f1f4b-7a5d-7b5f-8d31-123456789abf", name: "新实例", authorization_code: "n".repeat(36) };
          clients.push(created); return route.fulfill({ status: 201, json: created });
        }
        if (path.endsWith(`/clients/${activeId}`) && request.method() === "PATCH") {
          assert.deepEqual(request.postDataJSON(), { name: "Renamed instance" });
          const target = clients.find(value => value.id === activeId);
          target.name = "Renamed instance";
          return route.fulfill({ json: target });
        }
        if (path.endsWith(`/clients/${pendingId}`) && request.method() === "DELETE") {
          const target = clients.find(value => value.id === pendingId);
          if (target?.status === "revoked") clients = clients.filter(value => value.id !== pendingId); else target.status = "revoked";
          return route.fulfill({ status: 204 });
        }
        if (path.endsWith("/ptz")) { ptz.push(request.postDataJSON().action); return route.fulfill({ status: 204 }); }
        if (path.endsWith("/cameras") && request.method() === "GET") return route.fulfill({ json: [camera] });
        if (path.endsWith("/events/event-1/ack")) { acknowledged = true; return route.fulfill({ status: 204 }); }
        if (path.endsWith("/events")) {
          const unacknowledged = url.searchParams.get("unacknowledged"); eventQueries.push(unacknowledged);
          return route.fulfill({ json: unacknowledged === "true" && acknowledged ? [] : [{ id: "event-1", camera_id: camera.id, kind: "camera.status", severity: "info", message: "验收事件", acknowledged_at: acknowledged ? time : null, created_at: time }] });
        }
        if (path.endsWith("/media/operations")) return route.fulfill({ json: [] });
        if (path.endsWith("/audit")) {
          if (failAudit) { failAudit = false; return route.fulfill({ status: 500, json: { code: "platform.internal", message: "SECRET database path", retryable: false, request_id: "audit-failure-123" } }); }
          return route.fulfill({ json: [{ id: "audit-1", user_id: administratorId, action: "camera.updated", entity_type: "camera", entity_id: camera.id, details: { generation: 1 }, created_at: time }] });
        }
        if (path.endsWith("/recordings")) { assert.equal(url.searchParams.get("camera_id"), camera.id); return route.fulfill({ json: [{ start: time, duration: 60 }] }); }
        throw new Error(`Unexpected API request ${request.method()} ${path}`);
      });
      await page.goto(`http://127.0.0.1:${address.port}/#instances`);
      const instanceTable = page.getByRole("table", { name: "摄像机实例列表" });
      const statistics = page.getByRole("table", { name: "实例统计" });
      await expect(statistics.getByRole("columnheader")).toHaveText(["统计项", "总数 / 在线"]);
      await expect(statistics).not.toContainText("待配对实例");
      await expect(statistics.getByRole("row").nth(1).locator("th, td")).toHaveText(["总数", "2 / 1"]);
      await expect(instanceTable.getByRole("link", { name: `选择实例 ${activeInstance.name}`, exact: true })).toBeVisible();
      const activeInstanceLink = instanceTable.getByRole("link", { name: `选择实例 ${activeInstance.name}`, exact: true });
      const instanceLinkStyle = await activeInstanceLink.evaluate(element => ({
        color: getComputedStyle(element).color,
        parentColor: getComputedStyle(element.parentElement).color,
        decoration: getComputedStyle(element).textDecorationLine,
      }));
      assert.equal(instanceLinkStyle.color, instanceLinkStyle.parentColor);
      assert.equal(instanceLinkStyle.decoration, "none");
      await expect(instanceTable.getByRole("columnheader")).toHaveText(["实例名称", "配对状态", "摄像机状态", "厂商 / 型号", "操作", "删除"]);
      await expect(instanceTable).not.toContainText(activeInstance.id);
      await expect(instanceTable).not.toContainText(activeInstance.authorization_code);
      assert.ok((await instanceTable.locator("th, td").evaluateAll(elements => elements.map(element => getComputedStyle(element).textAlign))).every(value => value === "left"));
      await assertColumnContentAlignment(instanceTable);
      assert.ok((await instanceTable.locator(".sarmg-actions").evaluateAll(elements => elements.map(element => getComputedStyle(element).justifyContent))).every(value => value === "flex-start"));
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.locator(".sarmg-instance-sidebar, .sarmg-instance-workspace")).toHaveCount(0);
      const menuToFirst = await page.evaluate(() => {
        const header = document.querySelector(".sarmg-page-header");
        const first = document.querySelector(".sentinel-business");
        if (!header || !first) throw new Error("Sentinel spacing fixture is incomplete");
        return first.getBoundingClientRect().top - header.getBoundingClientRect().bottom;
      });
      assert.ok(Math.abs(menuToFirst - 16) < 2, String(menuToFirst));
      await page.getByRole("button", { name: "取消配对", exact: true }).click();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByRole("cell", { name: "已撤销", exact: true })).toBeVisible();
      const pendingRow = instanceTable.locator("tbody tr").filter({ hasText: pendingInstance.name });
      await pendingRow.getByRole("button", { name: "删除", exact: true }).click();
      await pendingRow.getByRole("button", { name: "取消", exact: true }).click();
      await expect(pendingRow.getByRole("button", { name: "确认删除", exact: true })).toHaveCount(0);
      await pendingRow.getByRole("button", { name: "删除", exact: true }).click();
      await pendingRow.getByRole("button", { name: "确认删除", exact: true }).click();
      await expect(instanceTable.locator("tbody tr")).toHaveCount(1);
      await expect(page.getByRole("main").getByRole("button", { name: "新建实例", exact: true })).toHaveCount(0);
      await page.getByRole("banner").getByRole("button", { name: "新建实例", exact: true }).click();
      await expect(instanceTable.getByRole("link", { name: "选择实例 新实例", exact: true })).toBeVisible();
      const dismissNotifications = page.getByRole("button", { name: "关闭通知", exact: true });
      await expect(dismissNotifications.first()).toBeVisible();
      await expect(dismissNotifications).toHaveCount(0, { timeout: 7_000 });
      await instanceTable.getByRole("link", { name: `选择实例 ${activeInstance.name}`, exact: true }).click();
      await expect(page.getByRole("button", { name: "详细信息", exact: true })).toHaveAttribute("aria-pressed", "true");
      const pairingDetails = page.getByRole("region", { name: "配对账户信息" });
      await expect(pairingDetails).toContainText(activeInstance.id);
      await expect(pairingDetails).toContainText(activeInstance.authorization_code);
      await page.getByLabel("实例名称", { exact: true }).fill("Renamed instance");
      await page.getByRole("button", { name: "保存名称", exact: true }).click();
      await expect(pairingDetails).toContainText("Renamed instance");
      await expect(page.getByRole("region", { name: "摄像机状态" })).toContainText("0.3.0");
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.locator(".sarmg-instance-sidebar, .sarmg-instance-workspace")).toHaveCount(0);
      await page.getByRole("button", { name: "查看与控制", exact: true }).click();
      const movement = page.getByRole("button", { name: "云台向上", exact: true });
      await movement.focus(); await page.keyboard.down("Space"); await page.keyboard.up("Space");
      await expect.poll(() => ptz.slice()).toEqual(["move", "stop"]);
      await movement.focus(); await page.keyboard.down("Enter");
      await page.evaluate(() => window.dispatchEvent(new Event("blur")));
      await page.keyboard.up("Enter");
      await expect.poll(() => ptz.slice()).toEqual(["move", "stop", "move", "stop"]);
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: "查询录像", exact: true }).click();
      await expect(page.getByRole("button", { name: /1分0秒/ })).toBeVisible();
      await page.getByRole("button", { name: "日志", exact: true }).click();
      await expect(page.getByRole("button", { name: "查询录像", exact: true })).toHaveCount(0);
      const filter = page.getByRole("checkbox", { name: "仅显示未确认事件", exact: true });
      const filterBox = await filter.boundingBox();
      assert.ok(filterBox.width <= 24 && filterBox.height <= 24);
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByRole("cell", { name: "已确认", exact: true })).toBeVisible();
      holdSystem = true;
      await page.getByRole("group", { name: "全局操作" }).getByRole("button", { name: "刷新", exact: true }).click();
      await expect.poll(() => typeof releaseSystem).toBe("function");
      await filter.check(); await expect(filter).toBeChecked();
      releaseSystem();
      await expect.poll(() => eventQueries.includes("true")).toBe(true);
      await expect(page.getByText("没有事件", { exact: true })).toBeVisible();
      const queriesBeforeUncheck = eventQueries.length;
      await filter.uncheck();
      await expect.poll(() => eventQueries.slice(queriesBeforeUncheck).includes(null), { timeout: 10_000 }).toBe(true);
      await expect(page.getByRole("cell", { name: "已确认", exact: true })).toBeVisible();
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.locator(".sarmg-instance-sidebar, .sarmg-instance-workspace")).toHaveCount(0);
      await expect(page.getByRole("banner").locator('.sarmg-product-identity')).toHaveText("Sentinel Monitor");
      await expect(page.getByText("更新摄像头", { exact: true })).toBeVisible();
      await expect(page.getByRole("heading", { name: "媒体协调操作", exact: true })).toBeVisible();
      await expect(page.getByRole("table", { name: "系统状态", exact: true })).toHaveCount(0);
      failAudit = true;
      await page.getByRole("group", { name: "全局操作" }).getByRole("button", { name: "刷新", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText("audit-failure-123");
      await expect(page.locator("body")).not.toContainText("SECRET");
      await page.getByRole("alert").getByRole("button", { name: "重试" }).click();
      await expect(page.getByText("更新摄像头", { exact: true })).toBeVisible();
      await expect(page.getByRole("heading", { name: "管理员账号", exact: true })).toHaveCount(0);
      await expect(page.getByRole("table", { name: "管理员账号", exact: true })).toHaveCount(0);

      for (const theme of ["light", "dark"]) {
        if (await page.locator("html").getAttribute("data-theme") !== theme) await page.getByRole("button", { name: /切换到.*模式/ }).click();
        assert.deepEqual((await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze()).violations, []);
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      }
      assert.ok(paths.some(path => path.endsWith("/system/status")));
      assert.ok(paths.some(path => path.endsWith("/media/operations")));
      assert.ok(!paths.some(path => path.includes("/users")));
      await checkWebLanguage(page, {"routes":[["instances","Instance list"],["logs","Logs"]],"names":["新实例","验收摄像头","门口摄像机实例","仓库摄像机","测试现场"]});
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: camera-instance list/details/recordings/logs, PTZ stop, modal focus and mobile WCAG AA passed`);
      await context.close();
    } finally { await browser.close(); }
  }
} finally { await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve())); }
