import { t } from "./i18n.js";

const STATUS_META = {
  protected: { badge: "status_protected", tone: "success", dot: "protected" },
  degraded: { badge: "status_degraded", tone: "warning", dot: "degraded" },
  vpn_unavailable: { badge: "status_vpn_unavailable", tone: "danger", dot: "failed" },
  non_network_failure: { badge: "status_non_network_failure", tone: "danger", dot: "failed" },
  paused: { badge: "status_paused", tone: "warning", dot: "degraded" },
};

const SOURCE_LABELS = {
  manual: "Manual",
  system_pac: "System PAC",
  system_proxy: "System proxy",
  environment: "Environment",
  known_loopback: "Known local port",
};

export function formatLatency(value) {
  if (!Number.isFinite(value) || value < 0) return "—";
  if (value < 1000) return `${Math.round(value)} ms`;
  return `${(value / 1000).toFixed(value >= 10_000 ? 0 : 1)} s`;
}

export function formatRemote(remote = {}, locale = "zh") {
  if (remote.online) return t(locale, "remote_online");
  if (remote.supported) return t(locale, "remote_pending");
  return t(locale, "remote_unsupported");
}

export function statusView(status = {}, locale = "zh") {
  const key = STATUS_META[status.status] ? status.status : "unknown";
  const meta = STATUS_META[key]
    ? { ...STATUS_META[key], badge: t(locale, STATUS_META[key].badge) }
    : { badge: t(locale, "status_unknown"), tone: "neutral", dot: "unknown" };
  const fallback = { title: t(locale, `${key}_title`), detail: t(locale, `${key}_detail`) };
  const backendGuidance = status.guidance || {};
  const guidance = {
    ...fallback,
    ...(locale === "zh" ? backendGuidance : {}),
    action: backendGuidance.action,
    action_label: backendGuidance.action
      ? t(locale, `action_${backendGuidance.action}`)
      : backendGuidance.action_label,
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
    diagnosticTitle: status.last_failure
      ? t(locale, "recent_diagnostic", { type: status.last_failure.class })
      : t(locale, "next_step"),
    diagnosticMessage: status.last_failure
      ? `${status.last_failure.summary}\n\n${guidance.detail}`
      : guidance.detail,
    relay: status.listen || "—",
    upstream: active?.candidate?.label || t(locale, "source_unknown"),
    upstreamSource: SOURCE_LABELS[active?.candidate?.source] || t(locale, "source_unknown"),
    latency: formatLatency(active?.latency_ms),
    remote: formatRemote(status.remote_control, locale),
    pauseLabel: status.paused ? t(locale, "resume_protection") : t(locale, "pause"),
  };
}
