import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";

import { formatLatency, formatRemote, statusView } from "./state.js";

test("protected state keeps the interface calm and surfaces its healthy route", () => {
  const view = statusView({
    status: "protected",
    listen: "127.0.0.1:17890",
    active_upstream: {
      latency_ms: 486,
      candidate: { label: "System PAC 127.0.0.1:7897", source: "system_pac" },
    },
    remote_control: { supported: true, online: false },
    guidance: { title: "已保护", detail: "连接稳定" },
  });
  assert.equal(view.meta.tone, "success");
  assert.equal(view.showNotice, false);
  assert.equal(view.showHeroAction, false);
  assert.equal(view.upstream, "System PAC 127.0.0.1:7897");
  assert.equal(view.latency, "486 ms");
  assert.equal(view.remote, "待连接");
});

test("VPN-unavailable state receives a high-priority recovery action", () => {
  const view = statusView({
    status: "vpn_unavailable",
    guidance: {
      title: "VPN 未启动",
      detail: "请启动 VPN 后重新检测。",
      action: "refresh",
      action_label: "重新检测",
    },
  });
  assert.equal(view.meta.tone, "danger");
  assert.equal(view.showNotice, true);
  assert.equal(view.showHeroAction, true);
  assert.equal(view.guidance.action, "refresh");
});

test("non-network guidance preserves the backend-recommended Codex action", () => {
  const view = statusView({
    status: "non_network_failure",
    last_failure: { class: "authentication", summary: "401 Unauthorized" },
    guidance: {
      title: "这是登录或账户问题，不是 VPN 问题",
      detail: "请重新登录 Codex。",
      action: "open_codex",
      action_label: "打开 Codex",
    },
  });
  assert.equal(view.meta.tone, "danger");
  assert.match(view.diagnosticMessage, /401 Unauthorized/);
  assert.equal(view.guidance.action, "open_codex");
});

test("formatters avoid misleading technical placeholders", () => {
  assert.equal(formatLatency(undefined), "—");
  assert.equal(formatLatency(1040), "1.0 s");
  assert.equal(formatRemote({ online: true }), "在线");
  assert.equal(formatRemote({ supported: false }), "当前版本不支持");
});

test("UI contract keeps the status-led structure and all functional controls", async () => {
  const [html, script, css] = await Promise.all([
    readFile(new URL("./index.html", import.meta.url), "utf8"),
    readFile(new URL("./app.js", import.meta.url), "utf8"),
    readFile(new URL("./styles.css", import.meta.url), "utf8"),
  ]);
  for (const id of ["status-card", "status-title", "hero-action", "refresh", "open-codex", "diagnostic-action", "upstream-form"]) {
    assert.match(html, new RegExp(`id="${id}"`));
  }
  assert.match(html, /type="module" src="app\.js"/);
  assert.match(html, /assets\/brand-logo-v1\.png/);
  assert.match(script, /statusView\(status\)/);
  assert.match(script, /hero-action/);
  assert.match(css, /color-scheme: light/);
  assert.match(css, /\.status-card/);
});
