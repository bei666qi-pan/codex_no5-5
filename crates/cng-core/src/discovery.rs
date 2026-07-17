use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use regex::Regex;
use tokio::process::Command;
use url::Url;

use crate::config::{GuardConfig, GuardMode};
use crate::model::{CandidateSource, UpstreamCandidate};

const COMMON_PORTS: &[u16] = &[
    7890, 7891, 7897, 1080, 10808, 10809, 6152, 6153, 20170, 33210,
];
const KEYCHAIN_SERVICE: &str = "dev.codex-network-guard.upstream";
const KEYCHAIN_ACCOUNT: &str = "upstream";

pub async fn discover(config: &GuardConfig) -> Result<Vec<UpstreamCandidate>> {
    let mut candidates = Vec::new();

    if let Some(value) = manual_upstream(config).await? {
        add_url(
            &mut candidates,
            &value,
            CandidateSource::Manual,
            "Manual upstream",
        );
    }
    if config.mode == GuardMode::Manual {
        return Ok(deduplicate(candidates));
    }

    #[cfg(target_os = "macos")]
    candidates.extend(discover_macos_system_proxy().await.unwrap_or_default());
    #[cfg(target_os = "windows")]
    candidates.extend(discover_windows_system_proxy().await.unwrap_or_default());
    candidates.extend(discover_environment());
    candidates.extend(common_loopback_candidates());
    Ok(deduplicate(candidates))
}

fn discover_environment() -> Vec<UpstreamCandidate> {
    let mut values = Vec::new();
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(value) = std::env::var(key) {
            add_url(
                &mut values,
                &value,
                CandidateSource::Environment,
                format!("Environment {key}"),
            );
        }
    }
    values
}

fn common_loopback_candidates() -> Vec<UpstreamCandidate> {
    let mut values = Vec::new();
    for port in COMMON_PORTS {
        add_url(
            &mut values,
            &format!("http://127.0.0.1:{port}"),
            CandidateSource::KnownLoopback,
            format!("Loopback HTTP {port}"),
        );
        add_url(
            &mut values,
            &format!("socks5h://127.0.0.1:{port}"),
            CandidateSource::KnownLoopback,
            format!("Loopback SOCKS5 {port}"),
        );
    }
    values
}

#[cfg(target_os = "macos")]
async fn discover_macos_system_proxy() -> Result<Vec<UpstreamCandidate>> {
    let output = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .stdin(Stdio::null())
        .output()
        .await
        .context("run scutil --proxy")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let fields = parse_scutil_fields(&text);
    let mut candidates = Vec::new();

    if fields.get("ProxyAutoConfigEnable").map(String::as_str) == Some("1")
        && let Some(pac_url) = fields.get("ProxyAutoConfigURLString")
    {
        candidates.extend(discover_pac(pac_url).await.unwrap_or_default());
    }

    for (enabled, host_key, port_key, scheme) in [
        ("HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http"),
        ("HTTPEnable", "HTTPProxy", "HTTPPort", "http"),
        ("SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks5h"),
    ] {
        if fields.get(enabled).map(String::as_str) != Some("1") {
            continue;
        }
        if let (Some(host), Some(port)) = (fields.get(host_key), fields.get(port_key)) {
            add_url(
                &mut candidates,
                &format!("{scheme}://{host}:{port}"),
                CandidateSource::SystemProxy,
                format!("macOS system proxy {host}:{port}"),
            );
        }
    }
    Ok(candidates)
}

#[cfg(target_os = "macos")]
fn parse_scutil_fields(text: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let pattern = Regex::new(r"(?m)^\s*([A-Za-z0-9]+)\s*:\s*(.+?)\s*$").expect("valid regex");
    for capture in pattern.captures_iter(text) {
        fields.insert(capture[1].to_string(), capture[2].to_string());
    }
    fields
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn discover_pac(pac_url: &str) -> Result<Vec<UpstreamCandidate>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .build()?;
    let script = client
        .get(pac_url)
        .send()
        .await
        .context("download PAC")?
        .error_for_status()
        .context("PAC returned an error")?
        .text()
        .await?;
    Ok(parse_pac_candidates(&script))
}

#[cfg(target_os = "windows")]
async fn discover_windows_system_proxy() -> Result<Vec<UpstreamCandidate>> {
    let output = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .context("query Windows Internet Settings proxy configuration")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let fields = parse_windows_registry_values(&String::from_utf8_lossy(&output.stdout));
    let mut candidates = Vec::new();
    if let Some(pac_url) = fields.get("AutoConfigURL") {
        candidates.extend(discover_pac(pac_url).await.unwrap_or_default());
    }
    let explicit_enabled = fields
        .get("ProxyEnable")
        .is_some_and(|value| matches!(value.trim(), "1" | "0x1"));
    if explicit_enabled && let Some(proxy) = fields.get("ProxyServer") {
        candidates.extend(parse_windows_proxy_server(proxy));
    }
    Ok(candidates)
}

#[cfg(any(target_os = "windows", test))]
fn parse_windows_registry_values(text: &str) -> HashMap<String, String> {
    let pattern = Regex::new(r"(?m)^\s*([^\s]+)\s+REG_[A-Z_]+\s+(.+?)\s*$")
        .expect("valid Windows registry regex");
    pattern
        .captures_iter(text)
        .map(|capture| (capture[1].to_string(), capture[2].to_string()))
        .collect()
}

#[cfg(any(target_os = "windows", test))]
fn parse_windows_proxy_server(value: &str) -> Vec<UpstreamCandidate> {
    let mut candidates = Vec::new();
    for segment in value
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let (kind, endpoint) = segment
            .split_once('=')
            .map_or(("http", segment), |(kind, endpoint)| {
                (kind, endpoint.trim())
            });
        let scheme = match kind.trim().to_ascii_lowercase().as_str() {
            "socks" | "socks4" | "socks5" => "socks5h",
            "https" => "https",
            _ => "http",
        };
        add_url(
            &mut candidates,
            &format!("{scheme}://{endpoint}"),
            CandidateSource::SystemProxy,
            format!("Windows system proxy {endpoint}"),
        );
    }
    deduplicate(candidates)
}

pub fn parse_pac_candidates(script: &str) -> Vec<UpstreamCandidate> {
    let pattern = Regex::new(r"(?i)\b(PROXY|HTTPS|SOCKS5|SOCKS)\s+([A-Za-z0-9._\-\[\]:]+)")
        .expect("valid PAC regex");
    let mut candidates = Vec::new();
    for capture in pattern.captures_iter(script) {
        let scheme = match capture[1].to_ascii_uppercase().as_str() {
            "SOCKS" | "SOCKS5" => "socks5h",
            "HTTPS" => "https",
            _ => "http",
        };
        let endpoint = capture[2].trim_matches([';', '"', '\'']);
        add_url(
            &mut candidates,
            &format!("{scheme}://{endpoint}"),
            CandidateSource::SystemPac,
            format!("System PAC {endpoint}"),
        );
    }
    deduplicate(candidates)
}

fn add_url(
    output: &mut Vec<UpstreamCandidate>,
    value: &str,
    source: CandidateSource,
    label: impl Into<String>,
) {
    let normalized = if value.contains("://") {
        value.trim().to_string()
    } else {
        format!("http://{}", value.trim())
    };
    if let Ok(url) = Url::parse(&normalized)
        && let Some(candidate) = UpstreamCandidate::from_url(url, source, label)
    {
        output.push(candidate);
    }
}

fn deduplicate(values: Vec<UpstreamCandidate>) -> Vec<UpstreamCandidate> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|candidate| seen.insert(candidate.id.clone()))
        .collect()
}

async fn manual_upstream(config: &GuardConfig) -> Result<Option<String>> {
    if config.manual_upstream_keychain {
        return read_manual_upstream_keychain().await;
    }
    Ok(config.manual_upstream.clone())
}

#[cfg(target_os = "macos")]
pub async fn save_manual_upstream(value: &str) -> Result<bool> {
    let url = Url::parse(value).context("manual upstream must be a valid URL")?;
    if url.host_str().is_none() || url.port_or_known_default().is_none() {
        anyhow::bail!("manual upstream must contain a host and port");
    }
    if !url.username().is_empty() || url.password().is_some() {
        let status = Command::new("/usr/bin/security")
            .args([
                "add-generic-password",
                "-U",
                "-a",
                KEYCHAIN_ACCOUNT,
                "-s",
                KEYCHAIN_SERVICE,
                "-w",
                value,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;
        anyhow::ensure!(
            status.success(),
            "could not store proxy credentials in Keychain"
        );
        return Ok(true);
    }
    Ok(false)
}

#[cfg(not(target_os = "macos"))]
pub async fn save_manual_upstream(value: &str) -> Result<bool> {
    let url = Url::parse(value).context("manual upstream must be a valid URL")?;
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "credential storage is not implemented on this platform"
    );
    Ok(false)
}

#[cfg(target_os = "macos")]
async fn read_manual_upstream_keychain() -> Result<Option<String>> {
    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-a",
            KEYCHAIN_ACCOUNT,
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
        ])
        .stdin(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
async fn read_manual_upstream_keychain() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ordered_pac_candidates() {
        let values = parse_pac_candidates(
            r#"return "PROXY 127.0.0.1:7897; SOCKS5 127.0.0.1:1080; DIRECT";"#,
        );
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].source, CandidateSource::SystemPac);
        assert_eq!(values[0].url.scheme(), "http");
        assert_eq!(values[1].url.scheme(), "socks5h");
    }

    #[test]
    fn relay_endpoint_is_never_rediscovered() {
        let mut values = Vec::new();
        add_url(
            &mut values,
            "http://127.0.0.1:17890",
            CandidateSource::Environment,
            "loop",
        );
        assert!(values.is_empty());
    }

    #[test]
    fn parses_windows_proxy_settings() {
        let fields = parse_windows_registry_values(
            "    ProxyEnable    REG_DWORD    0x1\n    ProxyServer    REG_SZ    http=127.0.0.1:7890;socks=127.0.0.1:7891\n",
        );
        assert_eq!(fields.get("ProxyEnable"), Some(&"0x1".to_string()));
        let values = parse_windows_proxy_server(fields.get("ProxyServer").unwrap());
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].url.scheme(), "http");
        assert_eq!(values[1].url.scheme(), "socks5h");
    }
}
