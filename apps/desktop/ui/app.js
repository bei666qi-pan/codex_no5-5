import { statusView } from "./state.js";

const invoke = window.__TAURI__.core.invoke;
let current = null;

const $ = (id) => document.getElementById(id);

function setText(id, value) {
  $(id).textContent = value;
}

function render(status) {
  current = status;
  const view = statusView(status);
  document.body.dataset.tone = view.meta.tone;
  $("status-card").dataset.status = view.key;
  setText("status-badge", view.meta.badge);
  setText("status-title", view.guidance.title);
  setText("status-detail", view.guidance.detail);
  $("status-dot").className = `dot ${view.meta.dot}`;
  setText("relay", view.relay);
  setText("upstream", view.upstream);
  setText("upstream-source", view.upstreamSource);
  setText("latency", view.latency);
  setText("remote", view.remote);
  setText("pause", view.pauseLabel);

  const heroAction = $("hero-action");
  heroAction.textContent = view.guidance.action_label || "";
  heroAction.classList.toggle("hidden", !view.showHeroAction);
  $("open-codex").classList.toggle("primary", !view.showHeroAction);
  $("open-codex").classList.toggle("secondary", view.showHeroAction);

  $("diagnostic").classList.toggle("hidden", !view.showNotice);
  if (view.showNotice) {
    setText("diagnostic-class", view.diagnosticTitle);
    setText("diagnostic-message", view.diagnosticMessage);
    const action = $("diagnostic-action");
    action.textContent = view.guidance.action_label || "";
    action.classList.toggle("hidden", !view.guidance.action_label);
  }
}

function renderDaemonUnavailable(error) {
  document.body.dataset.tone = "danger";
  setText("status-badge", "需要启用守护进程");
  setText("status-title", "尚未开始保护 Codex");
  setText("status-detail", "完成一次检测即可让 Codex 固定通过当前 VPN 连接。");
  $("status-dot").className = "dot failed";
  $("onboarding").classList.remove("hidden");
  $("diagnostic").classList.remove("hidden");
  setText("diagnostic-class", "本地守护进程未运行");
  setText("diagnostic-message", String(error));
  $("diagnostic-action").classList.add("hidden");
}

async function load(method = "guard_status") {
  try {
    render(await invoke(method));
    $("onboarding").classList.add("hidden");
  } catch (error) {
    renderDaemonUnavailable(error);
  }
}

async function showOutput(value) {
  $("output").classList.remove("hidden");
  $("output").textContent = typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

async function runGuidanceAction() {
  const action = current?.guidance?.action;
  try {
    if (action === "refresh") return load("refresh_guard");
    if (action === "resume_protection") return render(await invoke("set_paused", { paused: false }));
    if (action === "open_codex") return invoke("open_codex");
    if (action === "wait") {
      const path = await invoke("export_diagnostic");
      return showOutput(`已导出脱敏诊断：${path}`);
    }
  } catch (error) {
    return showOutput(String(error));
  }
}

async function runQuietAction(task) {
  try {
    await task();
  } catch (error) {
    await showOutput(String(error));
  }
}

$("refresh").addEventListener("click", () => load("refresh_guard"));
$("hero-action").addEventListener("click", runGuidanceAction);
$("diagnostic-action").addEventListener("click", runGuidanceAction);
$("open-codex").addEventListener("click", () => runQuietAction(() => invoke("open_codex")));
$("remote-start").addEventListener("click", () => runQuietAction(async () => showOutput(await invoke("remote_action", { action: "start" }))));
$("remote-pair").addEventListener("click", () => runQuietAction(async () => showOutput(await invoke("remote_action", { action: "pair" }))));
$("doctor").addEventListener("click", () => runQuietAction(async () => showOutput(await invoke("doctor"))));
$("export-diagnostic").addEventListener("click", () => runQuietAction(async () => {
  const path = await invoke("export_diagnostic");
  await showOutput(`已导出脱敏诊断：${path}`);
}));
$("pause").addEventListener("click", () => runQuietAction(async () => render(await invoke("set_paused", { paused: !current?.paused }))));
$("upstream-form").addEventListener("submit", (event) => runQuietAction(async () => {
  event.preventDefault();
  render(await invoke("set_upstream", { url: $("upstream-input").value }));
  await showOutput("本地代理已验证并启用。若 VPN 改端口，可随时恢复自动选择。");
}));
$("upstream-auto").addEventListener("click", () => runQuietAction(async () => {
  render(await invoke("set_upstream", { url: "auto" }));
  await showOutput("已恢复自动选择。CNG 会继续发现 VPN 当前的本地入口。");
}));
$("install").addEventListener("click", async () => {
  const button = $("install");
  button.disabled = true;
  button.textContent = "正在检测…";
  try {
    const probe = await invoke("onboarding_probe");
    $("step-codex").classList.toggle("done", probe.codex_found);
    $("step-vpn").classList.toggle("done", probe.healthy_count > 0);
    if (!probe.codex_found) throw new Error("未找到 Codex App 或 CLI");
    if (!probe.healthy_count) throw new Error("未找到可用 VPN 代理，请先启动 VPN");
    button.textContent = "正在启用…";
    const service = await invoke("install_guard");
    $("step-service").classList.toggle("done", service.running);
    $("restart-tip").classList.remove("hidden");
    $("migrate-legacy").classList.toggle("hidden", !service.legacy_guard_detected);
    await load();
  } catch (error) {
    await showOutput(String(error));
  } finally {
    button.disabled = false;
    button.textContent = "一键检测并启用";
  }
});
$("migrate-legacy").addEventListener("click", () => runQuietAction(async () => {
  await showOutput(await invoke("migrate_legacy_guard"));
  $("migrate-legacy").classList.add("hidden");
}));

load();
setInterval(() => load(), 10_000);
