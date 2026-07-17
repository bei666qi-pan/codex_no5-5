use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use percent_encoding::percent_decode_str;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_socks::tcp::Socks5Stream;
use tracing::{debug, info, warn};
use url::Url;

use crate::config::GuardConfig;
use crate::diagnostics::classify_error_text;
use crate::model::{
    CandidateHealth, HealthState, UpstreamCandidate, UpstreamKind, sort_candidates,
};

const MAX_HEADER_BYTES: usize = 32 * 1024;
const PROBE_HOST: &str = "chatgpt.com";
const PROBE_PORT: u16 = 443;

pub trait IoStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> IoStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}
pub type BoxedIo = Box<dyn IoStream>;

#[derive(Clone)]
pub struct ProxyRuntime {
    candidates: Arc<RwLock<Vec<CandidateHealth>>>,
    paused: Arc<AtomicBool>,
    direct_fallback: Arc<AtomicBool>,
    connect_timeout: Arc<RwLock<Duration>>,
}

impl ProxyRuntime {
    pub fn new(config: &GuardConfig) -> Self {
        Self {
            candidates: Arc::new(RwLock::new(Vec::new())),
            paused: Arc::new(AtomicBool::new(config.paused)),
            direct_fallback: Arc::new(AtomicBool::new(config.direct_fallback)),
            connect_timeout: Arc::new(RwLock::new(Duration::from_millis(
                config.connect_timeout_ms,
            ))),
        }
    }

    pub async fn candidates(&self) -> Vec<CandidateHealth> {
        self.candidates.read().await.clone()
    }

    pub async fn replace_candidates(&self, mut candidates: Vec<CandidateHealth>) {
        sort_candidates(&mut candidates);
        *self.candidates.write().await = candidates;
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_direct_fallback(&self, enabled: bool) {
        self.direct_fallback.store(enabled, Ordering::Relaxed);
    }

    pub async fn set_connect_timeout(&self, value: Duration) {
        *self.connect_timeout.write().await = value;
    }

    async fn connect_timeout(&self) -> Duration {
        *self.connect_timeout.read().await
    }
}

pub async fn run_proxy(listener: TcpListener, runtime: ProxyRuntime) -> Result<()> {
    let address = listener.local_addr()?;
    anyhow::ensure!(
        address.ip().is_loopback(),
        "proxy listener must be loopback-only"
    );
    info!(%address, "Codex relay is listening");
    loop {
        let (socket, peer) = listener.accept().await?;
        if !peer.ip().is_loopback() {
            warn!(%peer, "rejected non-loopback proxy client");
            continue;
        }
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_client(socket, runtime).await {
                debug!(error = %redact_error(&error.to_string()), "proxy connection ended");
            }
        });
    }
}

async fn handle_client(mut client: TcpStream, runtime: ProxyRuntime) -> Result<()> {
    if runtime.is_paused() {
        write_proxy_error(&mut client, 503, "Codex Network Guard is paused").await?;
        return Ok(());
    }

    let (header, extra) = read_http_header(&mut client).await?;
    let text = std::str::from_utf8(&header).context("proxy request header is not UTF-8")?;
    let first_line = text.lines().next().context("missing proxy request line")?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = parse_authority(target, 443)?;
        match connect_for_runtime(&runtime, &host, port).await {
            Ok((_candidate, mut upstream)) => {
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await?;
                if !extra.is_empty() {
                    upstream.write_all(&extra).await?;
                }
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            }
            Err(error) => {
                write_proxy_error(&mut client, 502, &redact_error(&error.to_string())).await?;
                return Err(error);
            }
        }
        return Ok(());
    }

    let url = Url::parse(target).context("plain HTTP proxy request must use an absolute URL")?;
    let host = url
        .host_str()
        .context("HTTP request URL has no host")?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    match connect_for_runtime(&runtime, &host, port).await {
        Ok((_candidate, mut upstream)) => {
            let rewritten = rewrite_plain_http_header(text, &url)?;
            upstream.write_all(rewritten.as_bytes()).await?;
            if !extra.is_empty() {
                upstream.write_all(&extra).await?;
            }
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        }
        Err(error) => {
            write_proxy_error(&mut client, 502, &redact_error(&error.to_string())).await?;
            return Err(error);
        }
    }
    Ok(())
}

async fn connect_for_runtime(
    runtime: &ProxyRuntime,
    host: &str,
    port: u16,
) -> Result<(Option<UpstreamCandidate>, BoxedIo)> {
    let candidates = runtime.candidates().await;
    let timeout_value = runtime.connect_timeout().await;
    let mut errors = Vec::new();
    for health in candidates {
        if health.state == HealthState::Down {
            continue;
        }
        match timeout(timeout_value, connect_tunnel(&health.candidate, host, port)).await {
            Ok(Ok(stream)) => return Ok((Some(health.candidate), stream)),
            Ok(Err(error)) => errors.push(redact_error(&error.to_string())),
            Err(_) => errors.push("connection timed out".to_string()),
        }
    }
    if runtime.direct_fallback.load(Ordering::Relaxed) {
        let stream = timeout(timeout_value, TcpStream::connect((host, port)))
            .await
            .context("direct fallback timed out")??;
        return Ok((None, Box::new(stream)));
    }
    anyhow::bail!(
        "no healthy proxy route{}",
        if errors.is_empty() {
            String::new()
        } else {
            format!(": {}", errors.join("; "))
        }
    )
}

pub async fn connect_tunnel(
    candidate: &UpstreamCandidate,
    host: &str,
    port: u16,
) -> Result<BoxedIo> {
    match candidate.kind {
        UpstreamKind::Http => {
            let stream = TcpStream::connect(
                candidate
                    .endpoint()
                    .context("proxy endpoint is incomplete")?,
            )
            .await
            .context("connect to HTTP proxy")?;
            http_connect(Box::new(stream), candidate, host, port).await
        }
        UpstreamKind::Https => {
            let endpoint = candidate
                .endpoint()
                .context("proxy endpoint is incomplete")?;
            let stream = TcpStream::connect(endpoint)
                .await
                .context("connect to HTTPS proxy")?;
            let proxy_host = candidate
                .url
                .host_str()
                .context("HTTPS proxy has no host")?;
            let server_name = ServerName::try_from(proxy_host.to_string())
                .context("invalid HTTPS proxy server name")?;
            let connector = TlsConnector::from(Arc::new(tls_client_config()?));
            let stream = connector
                .connect(server_name, stream)
                .await
                .context("TLS handshake with HTTPS proxy")?;
            http_connect(Box::new(stream), candidate, host, port).await
        }
        UpstreamKind::Socks5 => {
            let endpoint = candidate
                .endpoint()
                .context("SOCKS5 endpoint is incomplete")?;
            let stream = if candidate.has_credentials() {
                let username = decode(candidate.url.username());
                let password = decode(candidate.url.password().unwrap_or_default());
                Socks5Stream::connect_with_password(
                    endpoint.as_str(),
                    (host, port),
                    &username,
                    &password,
                )
                .await
                .context("SOCKS5 authenticated connect")?
            } else {
                Socks5Stream::connect(endpoint.as_str(), (host, port))
                    .await
                    .context("SOCKS5 connect")?
            };
            Ok(Box::new(stream))
        }
    }
}

async fn http_connect(
    mut stream: BoxedIo,
    candidate: &UpstreamCandidate,
    host: &str,
    port: u16,
) -> Result<BoxedIo> {
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut request = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n"
    );
    if candidate.has_credentials() {
        let credentials = format!(
            "{}:{}",
            decode(candidate.url.username()),
            decode(candidate.url.password().unwrap_or_default())
        );
        request.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            BASE64.encode(credentials)
        ));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;
    let (header, extra) = read_http_header(&mut stream).await?;
    let response = std::str::from_utf8(&header).context("proxy response is not UTF-8")?;
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .context("proxy returned an invalid status line")?;
    if status != 200 {
        anyhow::bail!("proxy CONNECT returned HTTP {status}");
    }
    if !extra.is_empty() {
        anyhow::bail!("proxy sent unexpected bytes after CONNECT response");
    }
    Ok(stream)
}

fn tls_client_config() -> Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    let result = rustls_native_certs::load_native_certs();
    for certificate in result.certs {
        let _ = roots.add(certificate);
    }
    anyhow::ensure!(
        !roots.is_empty(),
        "no native root certificates are available"
    );
    Ok(ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
}

pub async fn refresh_health(
    candidates: Vec<UpstreamCandidate>,
    previous: &[CandidateHealth],
    timeout_value: Duration,
) -> Vec<CandidateHealth> {
    let previous = previous
        .iter()
        .map(|value| (value.candidate.id.clone(), value.clone()))
        .collect::<HashMap<_, _>>();
    let checks = candidates.into_iter().map(|candidate| {
        let old = previous.get(&candidate.id).cloned();
        async move { check_candidate(candidate, old, timeout_value).await }
    });
    let mut health = futures::future::join_all(checks).await;
    sort_candidates(&mut health);
    health
}

async fn check_candidate(
    candidate: UpstreamCandidate,
    previous: Option<CandidateHealth>,
    timeout_value: Duration,
) -> CandidateHealth {
    let started = Instant::now();
    let result = timeout(timeout_value, probe_codex_route(&candidate)).await;
    match result {
        Ok(Ok(())) => CandidateHealth {
            candidate,
            state: HealthState::Healthy,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            checked_at: Utc::now(),
            consecutive_failures: 0,
            failure: None,
            detail: None,
        },
        value => {
            let detail = match value {
                Err(_) => "connection timed out".to_string(),
                Ok(Err(error)) => redact_error(&error.to_string()),
                Ok(Ok(())) => unreachable!(),
            };
            let failures = previous
                .as_ref()
                .map_or(1, |value| value.consecutive_failures.saturating_add(1));
            let state = if previous.as_ref().is_some_and(|value| {
                matches!(value.state, HealthState::Healthy | HealthState::Degraded)
            }) && failures < 2
            {
                HealthState::Degraded
            } else {
                HealthState::Down
            };
            CandidateHealth {
                candidate,
                state,
                latency_ms: None,
                checked_at: Utc::now(),
                consecutive_failures: failures,
                failure: Some(classify_error_text(&detail)),
                detail: Some(detail),
            }
        }
    }
}

async fn probe_codex_route(candidate: &UpstreamCandidate) -> Result<()> {
    let stream = connect_tunnel(candidate, PROBE_HOST, PROBE_PORT).await?;
    let connector = TlsConnector::from(Arc::new(tls_client_config()?));
    let server_name = ServerName::try_from(PROBE_HOST).context("invalid Codex probe host")?;
    let mut stream = connector
        .connect(server_name, stream)
        .await
        .context("TLS handshake through proxy")?;
    // An unauthenticated Upgrade request verifies HTTPS and WebSocket routing without sending a
    // Codex token or request body. Any syntactically valid HTTP response proves the route works.
    stream
        .write_all(
            b"GET /backend-api/codex HTTP/1.1\r\nHost: chatgpt.com\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: Y25nLXJvdXRlLXByb2Jl\r\nUser-Agent: codex-network-guard/0.1\r\n\r\n",
        )
        .await?;
    let (header, _) = read_http_header(&mut stream).await?;
    let response = std::str::from_utf8(&header).context("Codex probe response is not UTF-8")?;
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .context("Codex probe returned an invalid HTTP response")?;
    anyhow::ensure!(
        (100..600).contains(&status),
        "Codex probe returned HTTP {status}"
    );
    let _ = stream.shutdown().await;
    Ok(())
}

async fn read_http_header<S>(stream: &mut S) -> Result<(Vec<u8>, Vec<u8>)>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let count = stream.read(&mut chunk).await?;
        anyhow::ensure!(count > 0, "connection closed before HTTP headers completed");
        buffer.extend_from_slice(&chunk[..count]);
        anyhow::ensure!(buffer.len() <= MAX_HEADER_BYTES, "HTTP header is too large");
        if let Some(index) = find_header_end(&buffer) {
            return Ok((buffer[..index].to_vec(), buffer[index..].to_vec()));
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn parse_authority(value: &str, default_port: u16) -> Result<(String, u16)> {
    if let Ok(authority) = value.parse::<http::uri::Authority>() {
        let host = authority.host().trim_matches(['[', ']']).to_string();
        return Ok((host, authority.port_u16().unwrap_or(default_port)));
    }
    anyhow::bail!("invalid CONNECT authority")
}

fn rewrite_plain_http_header(header: &str, url: &Url) -> Result<String> {
    let first = header.lines().next().context("missing HTTP request line")?;
    let mut parts = first.split_whitespace();
    let method = parts.next().context("missing HTTP method")?;
    let _absolute = parts.next().context("missing HTTP target")?;
    let version = parts.next().unwrap_or("HTTP/1.1");
    let mut path = url.path().to_string();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let mut output = format!("{method} {path} {version}\r\n");
    for line in header.lines().skip(1) {
        if line.is_empty() {
            continue;
        }
        let name = line.split(':').next().unwrap_or_default();
        if name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("proxy-connection")
        {
            continue;
        }
        output.push_str(line);
        output.push_str("\r\n");
    }
    output.push_str("\r\n");
    Ok(output)
}

async fn write_proxy_error(client: &mut TcpStream, status: u16, message: &str) -> io::Result<()> {
    let safe = message.replace(['\r', '\n'], " ");
    let body = format!("Codex Network Guard: {safe}\n");
    let response = format!(
        "HTTP/1.1 {status} Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(response.as_bytes()).await
}

fn decode(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

pub fn redact_error(value: &str) -> String {
    let credential_pattern = regex::Regex::new(r"(?i)(https?://|socks5h?://)[^\s/@]+@").unwrap();
    let home = dirs::home_dir().map(|path| path.to_string_lossy().to_string());
    let redacted = credential_pattern
        .replace_all(value, "$1<redacted>@")
        .into_owned();
    match home {
        Some(home) => redacted.replace(&home, "~"),
        None => redacted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_and_ipv6_authority() {
        assert_eq!(
            parse_authority("chatgpt.com:443", 443).unwrap(),
            ("chatgpt.com".to_string(), 443)
        );
        assert_eq!(
            parse_authority("[::1]:8443", 443).unwrap(),
            ("::1".to_string(), 8443)
        );
    }

    #[test]
    fn rewrites_absolute_http_request_and_strips_proxy_credentials() {
        let header = "GET http://example.com/a?q=1 HTTP/1.1\r\nHost: example.com\r\nProxy-Authorization: Basic nope\r\n\r\n";
        let output =
            rewrite_plain_http_header(header, &Url::parse("http://example.com/a?q=1").unwrap())
                .unwrap();
        assert!(output.starts_with("GET /a?q=1 HTTP/1.1"));
        assert!(!output.contains("Proxy-Authorization"));
    }

    #[test]
    fn redacts_proxy_credentials_and_home() {
        let value = format!(
            "failed http://alice:secret@127.0.0.1:7890 at {}",
            dirs::home_dir().unwrap().display()
        );
        let output = redact_error(&value);
        assert!(!output.contains("secret"));
        assert!(!output.contains(&dirs::home_dir().unwrap().to_string_lossy().to_string()));
    }
}
