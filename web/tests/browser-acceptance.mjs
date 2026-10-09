import { checkAccountPage } from "./account-page.mjs";
import { rangeEditor, checkDateRangeValidation } from "./date-range.mjs";
import { checkWebLanguage } from "./language.mjs";
import assert from "node:assert/strict";
import { chromium, firefox, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { preview } from "vite";

const time = "2026-09-04T00:00:00Z";
const serverTime = "2026-09-04 08:00:00 +08:00";
async function assertLogControlsLayout(page) {
  const viewport = page.viewportSize();
  await expect(page.getByRole("table", { name: "监控事件", exact: true })).toBeVisible();
  for (const width of [1838, 360]) {
    await page.setViewportSize({ width, height: viewport.height });
    const layout = await page.locator(".xcss-log-date-controls").evaluate(element => {
      const label = element.querySelector("label").getBoundingClientRect();
      const button = element.querySelector("button").getBoundingClientRect();
      const input = element.querySelector(".xcss-date-range-fields").getBoundingClientRect();
      const controls = element.getBoundingClientRect();
      const textTop = node => { const range = document.createRange(); range.selectNodeContents(node); return range.getBoundingClientRect().top; };
      const textOffset = textTop(element.querySelector("button")) - textTop(element.querySelector("label"));
      return { textOffset, rowOffset: button.top - label.top, leftOffset: input.left - label.left,
        rightOffset: input.right - button.right, widthOffset: controls.width - input.width,
        rowGap: input.top - Math.max(label.bottom, button.bottom), right: controls.right };
    });
    const positions = await page.getByRole("region", { name: "日志筛选", exact: true }).evaluate(element => {
      const name = element.querySelector("p").getBoundingClientRect();
      const label = element.querySelector(".xcss-log-date-controls > label").getBoundingClientRect();
      const table = element.parentElement.querySelector(".xcss-table-scroll").getBoundingClientRect();
      const pager = element.parentElement.querySelector('nav[aria-label="监控事件分页"]').getBoundingClientRect();
      return { nameLeft: name.left-label.left, nameAbove: name.bottom<label.top, pagerBelow: pager.top>=table.bottom, rightAligned: Math.abs(pager.right-table.right)<1 };
    });
    assert.ok(Math.abs(positions.nameLeft)<1 && positions.nameAbove && positions.pagerBelow && positions.rightAligned, JSON.stringify(positions));
    assert.ok(Math.abs(layout.textOffset) <= 1 && Math.abs(layout.rowOffset) < 1 && Math.abs(layout.leftOffset) < 1 && Math.abs(layout.rightOffset) < 1 && Math.abs(layout.widthOffset) < 1, JSON.stringify(layout));
    assert.ok(layout.rowGap >= 7 && layout.rowGap <= 9 && layout.right <= width, JSON.stringify(layout));
  }
  await page.setViewportSize(viewport);
}
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
const secondId = "018f1f4b-7a5d-7b5f-8d31-123456789ac0";
const camera = { id: activeId, name: "验收摄像头", location: "测试现场", has_sub_stream: false, source_kind: "client", client_id: activeId, adapter_kind: "onvif", manufacturer: "Acme", model: "IPC-1", firmware_version: null, serial_number: null, capabilities: { video: "supported", main_stream: "supported", sub_stream: "unsupported", local_recording: "unsupported", server_recording: "supported", ptz: "supported", events: "unsupported", audio_input: "unsupported", audio_output: "unsupported" }, streams: [{ profile: "main", video_codec: null, audio_codec: null, width: null, height: null, frame_rate: null }], health_message: null, device_status: "disabled", storage_mode: "server", enabled: false, record_enabled: false, status: "disabled", last_seen_at: null, created_at: time, updated_at: time };
const activeInstance = { id: activeId, installation_id: "018f1f4b-7a5d-7b5f-8d31-123456789abe", name: "门口摄像机实例", client_version: "0.3.0", authorization_code: "a".repeat(36), status: "online", last_seen_at: time, created_at: time, updated_at: time };
const pendingInstance = { id: pendingId, installation_id: null, name: "待配对摄像机", client_version: null, authorization_code: "s".repeat(36), status: "pending", last_seen_at: null, created_at: time, updated_at: time };
const server = await preview({ preview: { host: "127.0.0.1", port: 0, strictPort: true } });
const address = server.httpServer.address();
assert.ok(address && typeof address === "object");
try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const context = await browser.newContext({ locale: "zh-CN", timezoneId: "America/Los_Angeles", viewport: { width: 360, height: 740 } });
      const page = await context.newPage();
      const errors = [], paths = [], ptz = [], eventQueries = [], logQueries = [], commandReceipts = new Map();
      let acknowledged = false, failAudit = false, holdSystem = false, releaseSystem = null, serverToday = "2026-09-04", clients = [{ ...activeInstance }, { ...pendingInstance }], cameras = [camera];
      const boundedName = `\uFEFF${"x".repeat(63)}`;
      const expectedNames = ["Renamed instance", "\uFEFFRenamed instance\uFEFF", "Renamed instance", boundedName, "Renamed instance"];
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/api/v1/**", async route => {
        const request = route.request(), url = new URL(request.url()), path = url.pathname;
        paths.push(path);
        if (path.endsWith("/auth/session")) return route.fulfill({ json: session });
        if (path.endsWith("/events/stream")) return route.fulfill({ status: 200, contentType: "text/event-stream", body: ": acceptance\n\n" });
        if (path.endsWith("/system/status")) {
          if (holdSystem) { holdSystem = false; await new Promise(resolve => { releaseSystem = resolve; }); releaseSystem = null; }
          return route.fulfill({ json: { service: "xcos", version: "0.3.0", database: "ok", media_service: "ok", cameras: { recording_configured: 0 }, server_time: time } });
        }
        if (path.endsWith("/logs/calendar")) return route.fulfill({ json: { today: serverToday } });
        if (request.method() !== "GET") assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
        if (path.endsWith("/clients") && request.method() === "GET") return route.fulfill({ json: clients });
        if (path.endsWith("/clients") && request.method() === "POST") {
          assert.deepEqual(request.postDataJSON(), { name: "新实例" });
          const created = { ...pendingInstance, id: "018f1f4b-7a5d-7b5f-8d31-123456789abf", name: "新实例", authorization_code: "n".repeat(36) };
          clients.push(created); return route.fulfill({ status: 201, json: created });
        }
        if (path.endsWith(`/clients/${activeId}`) && request.method() === "PATCH") {
          const name = expectedNames.shift();
          assert.deepEqual(request.postDataJSON(), { name });
          const target = clients.find(value => value.id === activeId);
          target.name = name;
          return route.fulfill({ json: target });
        }
        if (path.endsWith(`/clients/${pendingId}`) && request.method() === "DELETE") {
          const target = clients.find(value => value.id === pendingId);
          if (target?.status === "revoked") clients = clients.filter(value => value.id !== pendingId); else target.status = "revoked";
          return route.fulfill({ status: 204 });
        }
        if (path.endsWith("/ptz")) {
          ptz.push(request.postDataJSON().action);
          const command_id = `018f1f4b-7a5d-7b5f-8d31-${String(ptz.length).padStart(12,"0")}`;
          commandReceipts.set(command_id, {polls:0});
          return route.fulfill({status:202,json:{command_id,state:"queued"}});
        }
        if (path.includes("/commands/") && request.method() === "GET") {
          const command_id = path.split("/").at(-1), receipt = commandReceipts.get(command_id);
          assert.ok(receipt);
          receipt.polls += 1;
          return route.fulfill({json:{command_id,state:receipt.polls === 1 ? "awaiting_result" : "unconfirmed",error_code:receipt.polls === 1 ? null : "outcome_unknown"}});
        }
        if (path.endsWith("/stream-ticket")) return route.fulfill({ status: 503, json: { code: "media.unavailable", message: "unavailable" } });
        if (path.endsWith("/cameras") && request.method() === "GET") return route.fulfill({ json: cameras });
        if (path.endsWith("/events/event-1/ack")) { acknowledged = true; return route.fulfill({ status: 204 }); }
        const fulfillHistory = (rows) => {
          const start = Number(url.searchParams.get("cursor") ?? "0");
          const items = rows.slice(start, start + 100);
          return route.fulfill({ json: { format: "xcos-history-page-v1", items, next_cursor: start + items.length < rows.length ? String(start + items.length) : null } });
        };
        if (path.endsWith("/events/logs")) {
          const date = url.searchParams.get("date") ?? url.searchParams.get("start_date"), unacknowledged = url.searchParams.get("unacknowledged");
          logQueries.push({ path, date, end_date: url.searchParams.get("end_date"), limit: url.searchParams.get("limit"), offset: url.searchParams.get("offset") });
          eventQueries.push(unacknowledged);
          if (date === "2026-09-05") return fulfillHistory([]);
          if (date === "2026-09-03") return fulfillHistory(Array.from({ length: 220 }, (_, index) => ({ id: `past-event-${index}`, camera_id: camera.id, kind: "camera.status", severity: "info", message: "验收事件", acknowledged_at: null, created_at: "2026-09-02T16:00:00Z", server_created_at: "2026-09-03 00:00:00 +08:00" })));
          return fulfillHistory([{ id: "event-1", camera_id: camera.id, kind: "camera.status", severity: "info", message: "验收事件", acknowledged_at: acknowledged ? time : null, created_at: time, server_created_at: serverTime }]);
        }
        if (path.endsWith("/media/operations")) {
          const date = url.searchParams.get("date") ?? url.searchParams.get("start_date");
          logQueries.push({ path, date, end_date: url.searchParams.get("end_date"), limit: url.searchParams.get("limit"), offset: url.searchParams.get("offset") });
          return fulfillHistory(date === "2026-09-03" ? Array.from({ length: 160 }, (_, index) => ({ id: `past-operation-${index}`, camera_id: camera.id, generation: 1, kind: "reconcile_camera", state: "succeeded", reason: "camera.updated", requested_by: null, attempt: 1, max_attempts: 3, created_at: "2026-09-02T16:00:00Z", server_created_at: "2026-09-03 00:00:00 +08:00", started_at: null, finished_at: null, retry_at: null, error_code: null, error_message: null })) : []);
        }
        if (path.endsWith("/audit")) {
          if (failAudit) { failAudit = false; return route.fulfill({ status: 500, json: { code: "platform.internal", message: "SECRET database path", retryable: false, request_id: "audit-failure-123" } }); }
          const date = url.searchParams.get("date") ?? url.searchParams.get("start_date");
          logQueries.push({ path, date, end_date: url.searchParams.get("end_date"), limit: url.searchParams.get("limit"), offset: url.searchParams.get("offset") });
          return fulfillHistory(date === "2026-09-05" ? [] : date === "2026-09-03" ? Array.from({ length: 140 }, (_, index) => ({ id: `past-audit-${index}`, user_id: administratorId, action: "camera.updated", entity_type: "camera", entity_id: camera.id, details: { generation: 1, note: "x".repeat(60_000) }, created_at: "2026-09-02T16:00:00Z", server_created_at: "2026-09-03 00:00:00 +08:00" })) : [{ id: "audit-1", user_id: administratorId, action: "camera.updated", entity_type: "camera", entity_id: camera.id, details: { generation: 1 }, created_at: time, server_created_at: serverTime }]);
        }
        if (path.endsWith("/recordings")) { assert.equal(url.searchParams.get("camera_id"), camera.id); return route.fulfill({ json: [{ start: time, duration: 60 }] }); }
        throw new Error(`Unexpected API request ${request.method()} ${path}`);
      });
      await page.goto(`http://127.0.0.1:${address.port}/#instances`);
      const instanceTable = page.getByRole("table", { name: "摄像机实例列表" });
      const statistics = page.getByRole("table", { name: "实例统计" });
      await expect(page.locator("h2").filter({ hasText: /^(实例|实例列表|统计)$/ })).toHaveCount(0);
      await page.setViewportSize({ width: 1838, height: 900 });
      const statisticPositions = await statistics.locator("tbody tr").evaluateAll(rows => rows.map(row => ({ label: row.querySelector("th").getBoundingClientRect().top, value: row.querySelector("td").getBoundingClientRect().top })));
      assert.equal(statisticPositions.length, 4);
      assert.ok(statisticPositions.every(row => Math.abs(row.label - statisticPositions[0].label) < 1 && Math.abs(row.value - row.label) < 1), JSON.stringify(statisticPositions));
      await page.setViewportSize({ width: 360, height: 740 });
      await expect(statistics.getByRole("columnheader")).toHaveText(["统计项", "总数 / 在线"]);
      await expect(statistics).not.toContainText("待配对实例");
      await expect(statistics.getByRole("row").nth(1).locator("th, td")).toHaveText(["总数", "2 / 1"]);
      await expect(instanceTable.getByRole("link", { name: `选择实例 ${activeInstance.name}`, exact: true })).toBeVisible();
      await checkAccountPage(page);
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
      await instanceTable.getByRole("button", { name: "更换密码", exact: true }).first().click();
      const passwordDialog = page.getByRole("dialog", { name: "更换密码", exact: true });
      await expect(passwordDialog).toContainText("新密码重新配对");
      await passwordDialog.getByRole("button", { name: "取消", exact: true }).click();
      assert.ok((await instanceTable.locator("th, td").evaluateAll(elements => elements.map(element => getComputedStyle(element).textAlign))).every(value => value === "left"));
      await assertColumnContentAlignment(instanceTable);
      assert.ok((await instanceTable.locator(".xcss-actions").evaluateAll(elements => elements.map(element => getComputedStyle(element).justifyContent))).every(value => value === "flex-start"));
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.locator(".xcss-instance-sidebar, .xcss-instance-workspace")).toHaveCount(0);
      const menuToFirst = await page.evaluate(() => {
        const header = document.querySelector(".xcss-page-header");
        const first = document.querySelector(".xcos-business");
        if (!header || !first) throw new Error("Xcos spacing fixture is incomplete");
        return first.getBoundingClientRect().top - header.getBoundingClientRect().bottom;
      });
      assert.ok(Math.abs(menuToFirst) < 2, String(menuToFirst));
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
      await page.getByLabel("实例名称", { exact: true }).fill("\uFEFFRenamed instance\uFEFF");
      await page.getByRole("button", { name: "保存名称", exact: true }).click();
      await expect.poll(() => clients.find(value => value.id === activeId).name).toBe("\uFEFFRenamed instance\uFEFF");
      await page.getByLabel("实例名称", { exact: true }).fill("Renamed instance");
      await page.getByRole("button", { name: "保存名称", exact: true }).click();
      await expect.poll(() => clients.find(value => value.id === activeId).name).toBe("Renamed instance");
      await expect.poll(() => pairingDetails.locator("h2").evaluate(element => element.textContent)).toBe("Renamed instance");
      await page.getByLabel("实例名称", { exact: true }).fill(`\uFEFF${"x".repeat(64)}`);
      await expect(page.getByRole("button", { name: "保存名称", exact: true })).toBeDisabled();
      await expect(page.getByRole("region", { name: "实例设置" }).getByRole("alert")).toContainText("1–64 个字符");
      await page.getByLabel("实例名称", { exact: true }).fill(boundedName);
      await page.getByRole("button", { name: "保存名称", exact: true }).click();
      await expect.poll(() => pairingDetails.locator("h2").evaluate(element => element.textContent)).toBe(boundedName);
      await page.getByLabel("实例名称", { exact: true }).fill("Renamed instance");
      await page.getByRole("button", { name: "保存名称", exact: true }).click();
      await expect.poll(() => clients.find(value => value.id === activeId).name).toBe("Renamed instance");
      await expect(page.getByRole("region", { name: "摄像机状态" })).toContainText("0.3.0");
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.locator(".xcss-instance-sidebar, .xcss-instance-workspace")).toHaveCount(0);
      await page.getByRole("button", { name: "查看与控制", exact: true }).click();
      const movement = page.getByRole("button", { name: "云台向上", exact: true });
      await movement.focus(); await page.keyboard.down("Space"); await page.keyboard.up("Space");
      await expect.poll(() => ptz.slice()).toEqual(["move", "stop"]);
      await movement.focus(); await page.keyboard.down("Enter");
      await page.evaluate(() => window.dispatchEvent(new Event("blur")));
      await page.keyboard.up("Enter");
      await expect.poll(() => ptz.slice()).toEqual(["move", "stop", "move", "stop"]);
      await expect(page.getByRole("status").filter({hasText:"等待设备回执"})).toBeVisible();
      await expect(page.getByRole("status").filter({hasText:"结果未确认"})).toBeVisible();
      assert.equal(ptz.length, 4, "an unconfirmed physical command must not trigger automatic retries");
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: "查询录像", exact: true }).click();
      await expect(page.getByRole("button", { name: /1分0秒/ })).toBeVisible();
      clients.push({ ...activeInstance, id: secondId, name: "第二摄像机实例" });
      cameras = [...cameras, { ...camera, id: secondId, client_id: secondId, name: "第二摄像头" }];
      await page.getByRole("button", { name: "实例列表", exact: true }).click();
      await page.getByRole("banner").getByRole("button", { name: "刷新", exact: true }).click();
      await instanceTable.getByRole("link", { name: "选择实例 第二摄像机实例", exact: true }).click();
      await expect(page.getByRole("region", { name: "配对账户信息" })).toContainText("第二摄像机实例");
      const recordingCamera = page.getByRole("combobox", { name: "摄像头", exact: true });
      await expect(recordingCamera).toHaveValue(secondId);
      clients = clients.filter(client => client.id !== secondId);
      cameras = [camera];
      await page.getByRole("banner").getByRole("button", { name: "刷新", exact: true }).click();
      await expect(page.getByRole("region", { name: "配对账户信息" })).toContainText("Renamed instance");
      await expect(recordingCamera).toHaveValue(activeId);
      await page.getByRole("button", { name: "查询录像", exact: true }).click();
      await expect(page.getByRole("button", { name: /1分0秒/ })).toBeVisible();
      await page.getByRole("button", { name: "日志", exact: true }).click();
      await expect(page.getByRole("button", { name: "查询录像", exact: true })).toHaveCount(0);
      const serverDatePicker = rangeEditor(page, "xcos-log-date");
      await serverDatePicker.expectValue("2026-09-04");
      await assertLogControlsLayout(page);
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(1);
      await expect(page.getByText(serverTime, { exact: true }).first()).toBeVisible();
      await serverDatePicker.fill("2026-09-03");
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(100);
      await page.getByRole("navigation", { name: "监控事件分页" }).getByRole("button", { name: "下一页" }).click();
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(100);
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr").first()).toContainText("2026-09-03");
      await page.getByRole("navigation", { name: "监控事件分页" }).getByRole("button", { name: "下一页" }).click();
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(20);
      await page.getByRole("navigation", { name: "监控事件分页" }).getByRole("button", { name: "上一页" }).click();
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(100);
      await page.getByRole("navigation", { name: "监控事件分页" }).getByRole("button", { name: "下一页" }).click();
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(20);
      await page.getByRole("navigation", { name: "监控事件分页" }).getByRole("button", { name: "首页" }).click();
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(100);
      for (const [name, navigation, remainder] of [["媒体协调操作", "媒体操作分页", 60], ["审计日志", "审计日志分页", 40]]) {
        const table = page.getByRole("table", { name, exact: true });
        const pager = page.getByRole("navigation", { name: navigation, exact: true });
        await expect(table.locator("tbody tr")).toHaveCount(100);
        await expect(pager.getByRole("button", { name: "首页" })).toBeDisabled();
        await expect(pager.getByRole("button", { name: "上一页" })).toBeDisabled();
        await pager.getByRole("button", { name: "下一页" }).click();
        await expect(table.locator("tbody tr")).toHaveCount(remainder);
        await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(100);
        await pager.getByRole("button", { name: "上一页" }).click();
        await expect(table.locator("tbody tr")).toHaveCount(100);
        await pager.getByRole("button", { name: "下一页" }).click();
        await expect(table.locator("tbody tr")).toHaveCount(remainder);
        await pager.getByRole("button", { name: "首页" }).click();
        await expect(table.locator("tbody tr")).toHaveCount(100);
        const rectangles = await table.evaluate(element => ({ bottom: element.closest(".xcss-table-scroll").getBoundingClientRect().bottom }));
        const pagerBox = await pager.boundingBox();
        assert.ok(pagerBox.y >= rectangles.bottom, "log pagination follows its table");
      }
      assert.ok(logQueries.every(query => query.date && query.limit === null && query.offset === null));
      await checkDateRangeValidation(serverDatePicker, () => logQueries.length, "2026-09-03", "2026-09-04");
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(100);
      assert.equal(logQueries.at(-1).end_date, "2026-09-04");
      assert.ok(logQueries.slice(-3).every(query => query.end_date === "2026-09-04"));
      await serverDatePicker.fill("2026-09-04");
      await expect(page.getByRole("table", { name: "监控事件" }).locator("tbody tr")).toHaveCount(1);
      await expect(page.getByRole("checkbox", { name: "仅显示未确认事件", exact: true })).toHaveCount(0);
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByRole("cell", { name: "已确认", exact: true })).toBeVisible();
      holdSystem = true;
      const queriesBeforeRefresh = eventQueries.length;
      await page.getByRole("group", { name: "全局操作" }).getByRole("button", { name: "刷新", exact: true }).click();
      await expect.poll(() => typeof releaseSystem).toBe("function");
      releaseSystem();
      await expect.poll(() => eventQueries.length).toBeGreaterThan(queriesBeforeRefresh);
      assert.ok(eventQueries.every(value => value === null), "event logs never send the removed acknowledgement filter");
      await expect(page.getByRole("cell", { name: "已确认", exact: true })).toBeVisible();
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.locator(".xcss-instance-sidebar, .xcss-instance-workspace")).toHaveCount(0);
      await expect(page.getByRole("banner").locator('.xcss-product-identity')).toHaveText("xcos");
      await expect(page.getByText("更新摄像头", { exact: true })).toBeVisible();
      await expect(page.getByText("暂无媒体协调操作", { exact: true })).toBeVisible();
      await expect(page.getByRole("table", { name: "系统状态", exact: true })).toHaveCount(0);
      failAudit = true;
      await page.getByRole("group", { name: "全局操作" }).getByRole("button", { name: "刷新", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText("audit-failure-123");
      await expect(page.locator("body")).not.toContainText("SECRET");
      await page.getByRole("alert").getByRole("button", { name: "重试" }).click();
      await expect(page.getByText("更新摄像头", { exact: true })).toBeVisible();
      serverToday = "2026-09-05";
      await page.getByRole("button", { name: "实例列表", exact: true }).click();
      await page.getByRole("button", { name: "日志", exact: true }).click();
      await rangeEditor(page, "xcos-log-date").expectValue(serverToday);
      await expect(page.getByText("没有事件", { exact: true })).toBeVisible();
      assert.ok(logQueries.some(query => query.date === serverToday));
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
      cameras = [{ ...camera, enabled: true, device_status: "pending", status: "offline" }];
      await page.getByRole("button", { name: "实例列表", exact: true }).click();
      await page.getByRole("banner").getByRole("button", { name: "刷新", exact: true }).click();
      const activeRow = instanceTable.locator("tbody tr").filter({ hasText: "Renamed instance" });
      await expect(activeRow.getByRole("cell", { name: "等待检测", exact: true })).toBeVisible();
      cameras = [{ ...camera, enabled: true, device_status: "error", status: "offline", health_message: "media process exited; retrying" }];
      await page.getByRole("banner").getByRole("button", { name: "刷新", exact: true }).click();
      await expect(activeRow.getByRole("cell", { name: "设备异常", exact: true })).toBeVisible();
      if (engine === chromium) {
        await page.clock.install();
        cameras = [{ ...camera, enabled: true, device_status: "online", status: "online" }];
        await page.getByRole("banner").getByRole("button", { name: "刷新", exact: true }).click();
        await activeRow.getByRole("link", { name: "选择实例 Renamed instance", exact: true }).click();
        await page.clock.runFor(5_000);
        await expect(page.locator(".video-shell .video-state")).toHaveText("视频暂不可用");
        const attemptedTickets = paths.filter(path => path.endsWith("/stream-ticket")).length;
        await page.clock.runFor(16_000);
        await expect.poll(() => paths.filter(path => path.endsWith("/stream-ticket")).length).toBeGreaterThan(attemptedTickets);
        cameras = [camera];
        await page.getByRole("banner").getByRole("button", { name: "刷新", exact: true }).click();
        await expect(page.locator(".video-shell .video-state")).toHaveText("设备已停用");
      }
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: camera-instance list/details/recordings/logs, PTZ stop, modal focus and mobile WCAG AA passed`);
      await context.close();
    } finally { await browser.close(); }
  }
} finally { await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve())); }
