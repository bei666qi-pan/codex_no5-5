const invoke = window.__TAURI__.core.invoke;
let current = null;

const $ = (id) => document.getElementById(id);
const labels = {
  protected: ["已保护", "Codex 正在通过健康代理连接", "protected"],
  degraded: ["连接降级", "代理仍可用，但最近检测不稳定", "degraded"],
  vpn_unavailable: ["VPN 未启动", "未发现可用的 HTTP 或 SOCKS5 上游", "failed"],
  non_network_failure: ["Codex 非网络故障", "代理可用，但 Codex 自身返回了错误", "failed"],
  paused: ["保护已暂停", "Codex Network Guard 暂不转发连接", "unknown"],
};

function render(status) {
  current = status;
  const [title, detail, className] = labels[status.status] || ["未知状态", "请运行诊断", "unknown"];
  $("status-title").textContent = title;
  $("status-detail").textContent = detail;
  $("status-dot").className = `dot ${className}`;
  $("relay").textContent = status.listen || "—";
  $("upstream").textContent = status.active_upstream?.candidate?.label || "无可用上游";
  $("latency").textContent = status.active_upstream?.latency_ms != null ? `${status.active_upstream.latency_ms} ms` : "—";
  $("remote").textContent = status.remote_control?.online ? "在线" : status.remote_control?.supported ? "离线" : "当前版本不支持";
  $("pause").textContent = status.paused ? "恢复保护" : "暂停保护";
  if (status.last_failure) {
    $("diagnostic").classList.remove("hidden");
    $("diagnostic-class").textContent = `最近诊断 · ${status.last_failure.class}`;
    $("diagnostic-message").textContent = status.last_failure.summary;
  } else {
    $("diagnostic").classList.add("hidden");
  }
}

async function load(method = "guard_status") {
  try {
    render(await invoke(method));
    $("onboarding").classList.add("hidden");
  } catch (error) {
    $("status-title").textContent = "守护进程未运行";
    $("status-detail").textContent = String(error);
    $("status-dot").className = "dot failed";
    $("onboarding").classList.remove("hidden");
  }
}

async function showOutput(value) {
  $("output").classList.remove("hidden");
  $("output").textContent = typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

$("refresh").addEventListener("click", () => load("refresh_guard"));
$("open-codex").addEventListener("click", () => invoke("open_codex"));
$("remote-start").addEventListener("click", async () => showOutput(await invoke("remote_action", { action: "start" })));
$("remote-pair").addEventListener("click", async () => showOutput(await invoke("remote_action", { action: "pair" })));
$("doctor").addEventListener("click", async () => showOutput(await invoke("doctor")));
$("pause").addEventListener("click", async () => render(await invoke("set_paused", { paused: !current?.paused })));
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
    button.textContent = "正在安装自启…";
    const service = await invoke("install_guard");
    $("step-service").classList.toggle("done", service.running);
    $("restart-tip").classList.remove("hidden");
    $("migrate-legacy").classList.toggle("hidden", !service.legacy_guard_detected);
    await new Promise((resolve) => setTimeout(resolve, 800));
    await load();
  } catch (error) {
    await showOutput(String(error));
  } finally {
    button.disabled = false;
    button.textContent = "一键检测并启用";
  }
});
$("migrate-legacy").addEventListener("click", async () => {
  await showOutput(await invoke("migrate_legacy_guard"));
  $("migrate-legacy").classList.add("hidden");
});

load();
setInterval(() => load(), 10000);
