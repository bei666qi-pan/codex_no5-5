const STORAGE_KEY = "cng-ui-locale";
export const DEFAULT_LOCALE = "zh";

const MESSAGES = {
  zh: {
    app_title: "Codex Network Guard", brand_title: "连接守护", local_guard: "本机守护", language: "语言",
    status_checking: "正在检查", status_connecting: "正在检测连接…", status_connecting_detail: "正在连接本地守护进程",
    onboarding_eyebrow: "首次启用", onboarding_title: "让 Codex 固定通过当前 VPN", onboarding_detail: "只需完成一次检测和登录自启；不会修改系统代理或 VPN 配置。",
    step_codex: "找到 Codex", step_vpn: "验证 VPN 连接", step_service: "启用登录自启", install: "一键检测并启用", checking: "正在检测…", enabling: "正在启用…",
    restart_tip: "已启用。关闭并重新打开一次 Codex，之后无需重复操作。", migrate_legacy: "备份并停用旧版 Guard",
    overview_eyebrow: "连接概览", overview_title: "当前保护路径", fact_upstream: "当前上游", fact_relay: "本地入口", fact_latency: "检测延迟", fact_remote: "电脑端远程保活",
    next_step: "下一步", open_codex: "打开 Codex", refresh: "刷新检测", more_tools: "更多工具", more_tools_hint: "远程、诊断与保护控制",
    remote_start: "启用电脑端远程保活", remote_pair: "生成手机配对码", doctor: "查看诊断详情", export_diagnostic: "导出脱敏诊断", pause: "暂停保护",
    manual_proxy: "手动设置本地代理", manual_proxy_hint: "自动检测不到 VPN 时使用", manual_proxy_detail: "只填写 VPN 软件在本机暴露的 HTTP 或 SOCKS5 地址，不填写机场订阅链接。",
    proxy_input_label: "本地代理地址", proxy_test: "测试并使用", proxy_auto: "恢复自动选择", safety: "安全与隐私边界", safety_hint: "本机代理，不读取内容",
    safety_one: "仅监听本机 127.0.0.1:17890，不启用 TUN 或系统全局代理。", safety_two: "不解密 HTTPS，不读取 Codex 对话、代码、账号令牌或 VPN 配置。", safety_three: "VPN 不可用时默认阻止直连，避免断线时静默绕过代理。", safety_four: "导出诊断只包含状态、域名、错误类别和延迟；代理凭据会脱敏。",
    footer: "非 OpenAI 官方项目 · 仅保护 Codex 的网络连接", daemon_badge: "需要启用守护进程", daemon_title: "尚未开始保护 Codex", daemon_detail: "完成一次检测即可让 Codex 固定通过当前 VPN 连接。", daemon_not_running: "本地守护进程未运行",
    exported_diagnostic: "已导出脱敏诊断：{path}", upstream_enabled: "本地代理已验证并启用。若 VPN 改端口，可随时恢复自动选择。", upstream_auto_enabled: "已恢复自动选择。CNG 会继续发现 VPN 当前的本地入口。", codex_missing: "未找到 Codex App 或 CLI", vpn_missing: "未找到可用 VPN 代理，请先启动 VPN",
    status_protected: "连接受保护", status_degraded: "连接需要关注", status_vpn_unavailable: "需要 VPN", status_non_network_failure: "Codex 需要处理", status_paused: "保护已暂停", status_unknown: "正在检查",
    protected_title: "已保护", protected_detail: "Codex 正通过健康代理连接。", degraded_title: "连接降级", degraded_detail: "代理可用，但最近检测不稳定。", vpn_unavailable_title: "VPN 未启动", vpn_unavailable_detail: "没有发现可用的本地代理入口。", non_network_failure_title: "Codex 非网络故障", non_network_failure_detail: "代理可用，但 Codex 本身返回了错误。", paused_title: "保护已暂停", paused_detail: "守护进程暂不转发 Codex 连接。", unknown_title: "正在检查连接", unknown_detail: "正在等待本地守护进程返回状态。",
    remote_online: "在线", remote_pending: "待连接", remote_unsupported: "当前版本不支持", resume_protection: "恢复保护", recent_diagnostic: "最近诊断 · {type}", action_refresh: "重新检测", action_resume_protection: "恢复保护", action_open_codex: "打开 Codex", action_wait: "导出脱敏诊断", source_unknown: "未选择上游",
  },
  en: {
    app_title: "Codex Network Guard", brand_title: "Connection Guard", local_guard: "Local guard", language: "Language",
    status_checking: "Checking", status_connecting: "Checking your connection…", status_connecting_detail: "Connecting to the local guard",
    onboarding_eyebrow: "GET STARTED", onboarding_title: "Keep Codex on your current VPN", onboarding_detail: "One check and login launch setup. Your system proxy and VPN settings stay unchanged.",
    step_codex: "Find Codex", step_vpn: "Verify VPN route", step_service: "Enable launch at login", install: "Check and enable", checking: "Checking…", enabling: "Enabling…",
    restart_tip: "Enabled. Close and reopen Codex once; no repeat setup is needed.", migrate_legacy: "Back up and disable legacy Guard",
    overview_eyebrow: "CONNECTION OVERVIEW", overview_title: "Current protected route", fact_upstream: "ACTIVE UPSTREAM", fact_relay: "LOCAL RELAY", fact_latency: "CHECK LATENCY", fact_remote: "REMOTE KEEPALIVE",
    next_step: "Next step", open_codex: "Open Codex", refresh: "Refresh check", more_tools: "More tools", more_tools_hint: "Remote, diagnostics and protection",
    remote_start: "Start remote keepalive", remote_pair: "Create phone pairing code", doctor: "View diagnostics", export_diagnostic: "Export redacted diagnostics", pause: "Pause protection",
    manual_proxy: "Set a local proxy manually", manual_proxy_hint: "Use only if VPN auto-detection fails", manual_proxy_detail: "Enter the local HTTP or SOCKS5 address exposed by your VPN app, not a subscription URL.",
    proxy_input_label: "Local proxy address", proxy_test: "Test and use", proxy_auto: "Return to auto-select", safety: "Safety and privacy", safety_hint: "Local relay; content stays private",
    safety_one: "Listens only on 127.0.0.1:17890. No TUN or system-wide proxy is enabled.", safety_two: "Never decrypts HTTPS or reads Codex chats, code, tokens, or VPN configuration.", safety_three: "Blocks direct connections when the VPN route is unavailable.", safety_four: "Diagnostics contain only status, domain, error category, and latency; credentials are redacted.",
    footer: "Unofficial project · Protects Codex network connections only", daemon_badge: "Guard needs to start", daemon_title: "Codex is not protected yet", daemon_detail: "Complete one check to keep Codex on your current VPN route.", daemon_not_running: "Local guard is not running",
    exported_diagnostic: "Redacted diagnostics exported: {path}", upstream_enabled: "The local proxy was verified and enabled. Return to auto-select any time your VPN port changes.", upstream_auto_enabled: "Auto-select is back on. CNG will continue to find your VPN's current local entry point.", codex_missing: "Could not find Codex App or CLI", vpn_missing: "No usable local VPN proxy was found. Start your VPN first.",
    status_protected: "Connection protected", status_degraded: "Connection needs attention", status_vpn_unavailable: "VPN required", status_non_network_failure: "Codex needs attention", status_paused: "Protection paused", status_unknown: "Checking",
    protected_title: "Protected", protected_detail: "Codex is using a healthy proxy route.", degraded_title: "Connection degraded", degraded_detail: "The proxy works, but recent checks are unstable.", vpn_unavailable_title: "VPN is not running", vpn_unavailable_detail: "No usable local proxy entry point was found.", non_network_failure_title: "Non-network Codex issue", non_network_failure_detail: "The proxy works, but Codex returned an error.", paused_title: "Protection paused", paused_detail: "The guard is not forwarding Codex connections.", unknown_title: "Checking connection", unknown_detail: "Waiting for the local guard status.",
    remote_online: "Online", remote_pending: "Waiting", remote_unsupported: "Unsupported by this version", resume_protection: "Resume protection", recent_diagnostic: "Recent diagnostic · {type}", action_refresh: "Refresh check", action_resume_protection: "Resume protection", action_open_codex: "Open Codex", action_wait: "Export diagnostics", source_unknown: "No upstream selected",
  },
};

export function t(locale, key, values = {}) {
  const template = MESSAGES[locale]?.[key] ?? MESSAGES[DEFAULT_LOCALE][key] ?? key;
  return template.replace(/\{(\w+)\}/g, (_, name) => values[name] ?? `{${name}}`);
}

export function loadLocale() {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "zh" || stored === "en") return stored;
  return navigator.language?.toLowerCase().startsWith("zh") ? "zh" : "en";
}

export function saveLocale(locale) { localStorage.setItem(STORAGE_KEY, locale); }

export function applyStaticTranslations(locale, root = document) {
  root.documentElement.lang = locale === "zh" ? "zh-CN" : "en";
  root.title = t(locale, "app_title");
  for (const element of root.querySelectorAll("[data-i18n]")) element.textContent = t(locale, element.dataset.i18n);
  for (const element of root.querySelectorAll("[data-i18n-placeholder]")) element.placeholder = t(locale, element.dataset.i18nPlaceholder);
  for (const element of root.querySelectorAll("[data-i18n-aria-label]")) element.setAttribute("aria-label", t(locale, element.dataset.i18nAriaLabel));
}
