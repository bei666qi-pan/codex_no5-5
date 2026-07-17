use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::model::{DiagnosticEvent, FailureClass};
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
}
