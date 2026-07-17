export const STATUS_META = {
  protected: { badge: "连接受保护", tone: "success", dot: "protected" },
  degraded: { badge: "连接需要关注", tone: "warning", dot: "degraded" },
  vpn_unavailable: { badge: "需要 VPN", tone: "danger", dot: "failed" },
  non_network_failure: { badge: "Codex 需要处理", tone: "danger", dot: "failed" },
  paused: { badge: "保护已暂停", tone: "warning", dot: "degraded" },
};

const FALLBACK_GUIDANCE = {
  protected: { title: "已保护", detail: "Codex 正通过健康代理连接。" },
  degraded: { title: "连接降级", detail: "代理可用，但最近检测不稳定。" },
  vpn_unavailable: { title: "VPN 未启动", detail: "没有发现可用的本地代理入口。" },
  non_network_failure: { title: "Codex 非网络故障", detail: "代理可用，但 Codex 本身返回了错误。" },
  paused: { title: "保护已暂停", detail: "守护进程暂不转发 Codex 连接。" },
  unknown: { title: "正在检查连接", detail: "正在等待本地守护进程返回状态。" },
};

export function formatLatency(value) {
  if (!Number.isFinite(value) || value < 0) return "—";
  if (value < 1000) return `${Math.round(value)} ms`;
  return `${(value / 1000).toFixed(value >= 10_000 ? 0 : 1)} s`;
}

export function formatRemote(remote = {}) {
  if (remote.online) return "在线";
  if (remote.supported) return "待连接";
  return "当前版本不支持";
}

export function statusView(status = {}) {
  const key = STATUS_META[status.status] ? status.status : "unknown";
  const meta = STATUS_META[key] || { badge: "正在检查", tone: "neutral", dot: "unknown" };
  const fallback = FALLBACK_GUIDANCE[key];
  const guidance = {
    ...fallback,
    ...(status.guidance || {}),
  };
  const active = status.active_upstream;
  const showNotice = key !== "protected" || Boolean(status.last_failure);
  const showHeroAction = showNotice && Boolean(guidance.action_label && guidance.action);

  return {
    key,
    meta,
    guidance,
    showNotice,
    showHeroAction,
    diagnosticTitle: status.last_failure ? `最近诊断 · ${status.last_failure.class}` : "下一步",
    diagnosticMessage: status.last_failure
      ? `${status.last_failure.summary}\n\n${guidance.detail}`
      : guidance.detail,
    relay: status.listen || "—",
    upstream: active?.candidate?.label || "无可用上游",
    upstreamSource: active?.candidate?.source?.replaceAll("_", " ") || "未选择上游",
    latency: formatLatency(active?.latency_ms),
    remote: formatRemote(status.remote_control),
    pauseLabel: status.paused ? "恢复保护" : "暂停保护",
  };
}
