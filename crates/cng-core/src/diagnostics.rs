use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::model::{DiagnosticEvent, FailureClass, GuardStatusKind, UserGuidance};
use crate::proxy::redact_error;

pub fn classify_error_text(value: &str) -> FailureClass {
    let lower = value.to_ascii_lowercase();
    if contains_any(
        &lower,
        &[
            "err_proxy_connection_failed",
            "connection refused",
            "no healthy proxy",
            "proxy unavailable",
            "proxy listener",
        ],
    ) {
        FailureClass::ProxyUnavailable
    } else if contains_any(
        &lower,
        &[
            "name or service not known",
            "dns",
            "eai_fail",
            "no such host",
        ],
    ) {
        FailureClass::Dns
    } else if contains_any(
        &lower,
        &["certificate", "tls", "ssl", "unknown issuer", "invalid ca"],
    ) {
        FailureClass::Tls
    } else if contains_any(
        &lower,
        &[
            "proxy url scheme not supported",
            "proxy connect returned",
            "socks5",
            "proxy 407",
        ],
    ) {
        FailureClass::ProxyProtocol
    } else if contains_any(
        &lower,
        &["401", "403", "unauthorized", "forbidden", "authentication"],
    ) {
        FailureClass::Authentication
    } else if contains_any(&lower, &["429", "rate limit", "usage limit"]) {
        FailureClass::RateLimit
    } else if contains_any(
        &lower,
        &["500", "502", "503", "504", "internal server error"],
    ) {
        FailureClass::Server
    } else if contains_any(
        &lower,
        &[
            "websocket",
            "connection closed normally",
            "switching protocols",
        ],
    ) {
        FailureClass::WebSocket
    } else if contains_any(
        &lower,
        &[
            "sigkill",
            "app_server_connection.closed",
            "child process to exit",
        ],
    ) {
        FailureClass::AppServerCrash
    } else if contains_any(
        &lower,
        &[
            "custom tool call output is missing",
            "unknown conversation",
            "tool state",
        ],
    ) {
        FailureClass::ToolState
    } else if contains_any(&lower, &["timed out", "timeout"]) {
        FailureClass::Timeout
    } else {
        FailureClass::Unknown
    }
}

/// Turn a low-level state into a safe, concrete next step. This is deliberately
/// conservative: it never suggests disabling the VPN or allowing direct traffic.
pub fn guidance_for(status: GuardStatusKind, failure: Option<&DiagnosticEvent>) -> UserGuidance {
    match status {
        GuardStatusKind::Protected => UserGuidance {
            title: "已保护".into(),
            detail: "Codex 正通过固定本地入口连接健康代理。VPN 端口变化会在下一次连接时自动接管。".into(),
            action_label: Some("重新检测".into()),
            action: Some("refresh".into()),
        },
        GuardStatusKind::Degraded => UserGuidance {
            title: "连接降级".into(),
            detail: "代理仍可用，但最近检测不稳定。若持续出现重试，请在 VPN 客户端更换节点后重新检测。".into(),
            action_label: Some("重新检测".into()),
            action: Some("refresh".into()),
        },
        GuardStatusKind::Paused => UserGuidance {
            title: "保护已暂停".into(),
            detail: "暂停时守护进程不会替 Codex 转发连接。恢复后会继续禁止静默直连回退。".into(),
            action_label: Some("恢复保护".into()),
            action: Some("resume_protection".into()),
        },
        GuardStatusKind::VpnUnavailable => UserGuidance {
            title: "VPN 未启动或没有可用代理".into(),
            detail: "请先启动 VPN，并确认它提供系统 PAC、HTTP 或 SOCKS5 本地入口；CNG 已阻止 Codex 静默直连。".into(),
            action_label: Some("重新检测".into()),
            action: Some("refresh".into()),
        },
        GuardStatusKind::NonNetworkFailure => guidance_for_failure(failure),
    }
}

fn guidance_for_failure(failure: Option<&DiagnosticEvent>) -> UserGuidance {
    let class = failure
        .map(|event| event.class)
        .unwrap_or(FailureClass::Unknown);
    match class {
        FailureClass::Authentication => UserGuidance {
            title: "这是登录或账户问题，不是 VPN 问题".into(),
            detail: "代理连接正常，但 Codex 返回了 401 或 403。请重新登录 Codex 后再试。".into(),
            action_label: Some("打开 Codex".into()),
            action: Some("open_codex".into()),
        },
        FailureClass::RateLimit => UserGuidance {
            title: "这是服务限流，不是 VPN 问题".into(),
            detail: "Codex 返回了 429。反复重试通常无效，请稍后再试或检查账户使用限制。".into(),
            action_label: Some("查看脱敏诊断".into()),
            action: Some("wait".into()),
        },
        FailureClass::Server => UserGuidance {
            title: "Codex 服务暂时异常".into(),
            detail: "代理已可用，但服务端返回 5xx。请稍后重新检测，不建议反复切换代理。".into(),
            action_label: Some("查看脱敏诊断".into()),
            action: Some("wait".into()),
        },
        FailureClass::AppServerCrash | FailureClass::ToolState => UserGuidance {
            title: "Codex 本身需要恢复".into(),
            detail: "这不是网络故障。请重新打开 Codex；若仍出现，请导出脱敏诊断后提交反馈。".into(),
            action_label: Some("打开 Codex".into()),
            action: Some("open_codex".into()),
        },
        FailureClass::Dns | FailureClass::Tls | FailureClass::WebSocket | FailureClass::Timeout => UserGuidance {
            title: "网络路径仍不稳定".into(),
            detail: "请在 VPN 客户端更换节点或协议后重新检测；CNG 会继续保持固定入口，不需要重启它。".into(),
            action_label: Some("重新检测".into()),
            action: Some("refresh".into()),
        },
        FailureClass::ProxyUnavailable | FailureClass::ProxyProtocol => UserGuidance {
            title: "VPN 代理不可用".into(),
            detail: "请确认 VPN 已启动且本地端口没有被占用；如果自动检测失败，可在高级设置填写本地代理地址。".into(),
            action_label: Some("重新检测".into()),
            action: Some("refresh".into()),
        },
        FailureClass::Unknown => UserGuidance {
            title: "无法确认故障原因".into(),
            detail: "请导出脱敏诊断。它不包含 Codex 对话、账号令牌或代理密码。".into(),
            action_label: Some("导出脱敏诊断".into()),
            action: Some("wait".into()),
        },
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

pub fn latest_codex_diagnostic() -> Option<DiagnosticEvent> {
    let root = dirs::home_dir()?.join("Library/Logs/com.openai.codex");
    let mut files = collect_recent_logs(&root);
    files.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    for path in files.into_iter().rev().take(8) {
        if let Some(event) = scan_log(&path) {
            return Some(event);
        }
    }
    None
}

fn collect_recent_logs(root: &Path) -> Vec<PathBuf> {
    let mut output = Vec::new();
    let Ok(days) = fs::read_dir(root) else {
        return output;
    };
    for day in days.flatten() {
        let Ok(files) = fs::read_dir(day.path()) else {
            continue;
        };
        output.extend(
            files
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|value| value == "log")),
        );
    }
    output
}

fn scan_log(path: &Path) -> Option<DiagnosticEvent> {
    let content = fs::read_to_string(path).ok()?;
    let request_id_pattern = Regex::new(r"(?i)request id[=: ]+([a-z0-9-]+)").ok()?;
    for line in content.lines().rev().take(4_000) {
        let lower = line.to_ascii_lowercase();
        if !contains_any(
            &lower,
            &[
                "stream disconnected",
                "err_proxy",
                "error sending request",
                "websocket",
                "sigkill",
                "unauthorized",
                "rate limit",
                "custom tool call output is missing",
            ],
        ) {
            continue;
        }
        let class = classify_error_text(line);
        let request_id = request_id_pattern
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string());
        let summary = redact_error(line);
        return Some(DiagnosticEvent {
            timestamp: Utc::now(),
            class,
            summary: truncate(&summary, 800),
            source: path.file_name().map_or_else(
                || "codex log".to_string(),
                |value| value.to_string_lossy().into(),
            ),
            request_id,
        });
    }
    None
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect::<String>() + "…"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub generated_at: chrono::DateTime<Utc>,
    pub guard: crate::model::GuardStatus,
    pub codex: Option<serde_json::Value>,
    pub findings: Vec<DiagnosticEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_network_and_non_network_failures() {
        assert_eq!(
            classify_error_text("net::ERR_PROXY_CONNECTION_FAILED"),
            FailureClass::ProxyUnavailable
        );
        assert_eq!(
            classify_error_text("unexpected status 401 Unauthorized"),
            FailureClass::Authentication
        );
        assert_eq!(
            classify_error_text("Custom tool call output is missing for call id"),
            FailureClass::ToolState
        );
        assert_eq!(
            classify_error_text("websocket handshake returned 401"),
            FailureClass::Authentication
        );
    }

    #[test]
    fn guidance_never_recommends_direct_fallback() {
        let guidance = guidance_for(GuardStatusKind::VpnUnavailable, None);
        assert_eq!(guidance.action.as_deref(), Some("refresh"));
        assert!(guidance.detail.contains("阻止"));
    }

    #[test]
    fn authentication_is_not_presented_as_a_vpn_failure() {
        let event = DiagnosticEvent {
            timestamp: Utc::now(),
            class: FailureClass::Authentication,
            summary: "401".into(),
            source: "test".into(),
            request_id: None,
        };
        let guidance = guidance_for(GuardStatusKind::NonNetworkFailure, Some(&event));
        assert_eq!(guidance.action.as_deref(), Some("open_codex"));
        assert!(guidance.title.contains("账户"));
    }
}
