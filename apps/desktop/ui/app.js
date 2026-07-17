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
  const fallback = labels[status.status] || ["未知状态", "请运行诊断", "unknown"];
  const guidance = status.guidance || { title: fallback[0], detail: fallback[1] };
  const className = fallback[2];
  $("status-title").textContent = guidance.title;
  $("status-detail").textContent = guidance.detail;
  $("status-dot").className = `dot ${className}`;
  $("relay").textContent = status.listen || "—";
  $("upstream").textContent = status.active_upstream?.candidate?.label || "无可用上游";
  $("latency").textContent = status.active_upstream?.latency_ms != null ? `${status.active_upstream.latency_ms} ms` : "—";
  $("remote").textContent = status.remote_control?.online ? "在线" : status.remote_control?.supported ? "离线" : "当前版本不支持";
  $("pause").textContent = status.paused ? "恢复保护" : "暂停保护";
  const showGuidance = status.status !== "protected" || status.last_failure;
  if (showGuidance) {
    $("diagnostic").classList.remove("hidden");
    $("diagnostic-class").textContent = status.last_failure ? `最近诊断 · ${status.last_failure.class}` : "下一步";
    $("diagnostic-message").textContent = status.last_failure ? `${status.last_failure.summary}\n\n${guidance.detail}` : guidance.detail;
    const action = $("diagnostic-action");
    action.textContent = guidance.action_label || "";
    action.classList.toggle("hidden", !guidance.action_label);
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

$("refresh").addEventListener("click", () => load("refresh_guard"));
$("open-codex").addEventListener("click", () => invoke("open_codex"));
$("remote-start").addEventListener("click", async () => showOutput(await invoke("remote_action", { action: "start" })));
$("remote-pair").addEventListener("click", async () => showOutput(await invoke("remote_action", { action: "pair" })));
$("doctor").addEventListener("click", async () => showOutput(await invoke("doctor")));
$("export-diagnostic").addEventListener("click", async () => {
  try {
    const path = await invoke("export_diagnostic");
    await showOutput(`已导出脱敏诊断：${path}`);
  } catch (error) {
    await showOutput(String(error));
  }
});
$("pause").addEventListener("click", async () => render(await invoke("set_paused", { paused: !current?.paused })));
$("diagnostic-action").addEventListener("click", runGuidanceAction);
$("upstream-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    render(await invoke("set_upstream", { url: $("upstream-input").value }));
    await showOutput("本地代理已验证并启用。若 VPN 改端口，可随时点击“恢复自动选择”。");
  } catch (error) {
    await showOutput(`未能使用该代理：${error}`);
  }
});
$("upstream-auto").addEventListener("click", async () => {
  try {
    render(await invoke("set_upstream", { url: "auto" }));
    await showOutput("已恢复自动选择。CNG 会继续发现 VPN 当前的本地入口。");
  } catch (error) {
    await showOutput(String(error));
  }
});
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
