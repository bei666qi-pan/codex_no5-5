use std::cmp::Ordering;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use url::Url;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamKind {
    Http,
    Https,
    Socks5,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    Manual,
    SystemPac,
    SystemProxy,
    Environment,
    KnownLoopback,
}

impl CandidateSource {
    pub const fn priority(self) -> u8 {
        match self {
            Self::Manual => 0,
            Self::SystemPac => 1,
            Self::SystemProxy => 2,
            Self::Environment => 3,
            Self::KnownLoopback => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Degraded,
    #[default]
    Down,
    Unknown,
}

impl HealthState {
    const fn rank(self) -> u8 {
        match self {
            Self::Healthy => 0,
            Self::Degraded => 1,
            Self::Unknown => 2,
            Self::Down => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    ProxyUnavailable,
    Dns,
    Tls,
    ProxyProtocol,
    WebSocket,
    Authentication,
    RateLimit,
    Server,
    AppServerCrash,
    ToolState,
    Timeout,
    Unknown,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpstreamCandidate {
    pub id: String,
    pub kind: UpstreamKind,
    pub source: CandidateSource,
    #[serde(serialize_with = "serialize_redacted_url")]
    pub url: Url,
    pub label: String,
}

fn serialize_redacted_url<S>(url: &Url, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut redacted = url.clone();
    if !url.username().is_empty() {
        let _ = redacted.set_username("redacted");
    }
    if url.password().is_some() {
        let _ = redacted.set_password(Some("redacted"));
    }
    serializer.serialize_str(redacted.as_str())
}

impl std::fmt::Debug for UpstreamCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamCandidate")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("source", &self.source)
            .field("url", &self.redacted_url())
            .field("label", &self.label)
            .finish()
    }
}

impl UpstreamCandidate {
    pub fn from_url(url: Url, source: CandidateSource, label: impl Into<String>) -> Option<Self> {
        let kind = match url.scheme() {
            "http" => UpstreamKind::Http,
            "https" => UpstreamKind::Https,
            "socks" | "socks5" | "socks5h" => UpstreamKind::Socks5,
            _ => return None,
        };
        let host = url.host_str()?;
        let port = url.port_or_known_default()?;
        if host == "127.0.0.1" && port == crate::config::DEFAULT_LISTEN_PORT {
            return None;
        }
        let id = format!(
            "{}:{}:{}",
            source.priority(),
            kind as u8,
            endpoint_key(&url)
        );
        Some(Self {
            id,
            kind,
            source,
            url,
            label: label.into(),
        })
    }

    pub fn endpoint(&self) -> Option<String> {
        let host = self.url.host_str()?;
        let port = self.url.port_or_known_default()?;
        Some(if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        })
    }

    pub fn redacted_url(&self) -> String {
        let mut value = self.url.clone();
        let _ = value.set_username(if self.url.username().is_empty() {
            ""
        } else {
            "<redacted>"
        });
        let _ = value.set_password(self.url.password().map(|_| "<redacted>"));
        value.to_string()
    }

    pub fn has_credentials(&self) -> bool {
        !self.url.username().is_empty() || self.url.password().is_some()
    }
}

fn endpoint_key(url: &Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or_default()
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateHealth {
    pub candidate: UpstreamCandidate,
    pub state: HealthState,
    pub latency_ms: Option<u64>,
    pub checked_at: DateTime<Utc>,
    pub consecutive_failures: u32,
    pub failure: Option<FailureClass>,
    pub detail: Option<String>,
}

impl CandidateHealth {
    pub fn unknown(candidate: UpstreamCandidate) -> Self {
        Self {
            candidate,
            state: HealthState::Unknown,
            latency_ms: None,
            checked_at: Utc::now(),
            consecutive_failures: 0,
            failure: None,
            detail: None,
        }
    }
}

pub fn sort_candidates(candidates: &mut [CandidateHealth]) {
    candidates.sort_by(|left, right| {
        left.state
            .rank()
            .cmp(&right.state.rank())
            .then_with(|| {
                left.candidate
                    .source
                    .priority()
                    .cmp(&right.candidate.source.priority())
            })
            .then_with(|| match (left.latency_ms, right.latency_ms) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
    });
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GuardStatusKind {
    Protected,
    Degraded,
    #[default]
    VpnUnavailable,
    NonNetworkFailure,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GuardStatus {
    pub api_version: u32,
    pub status: GuardStatusKind,
    pub paused: bool,
    pub listen: String,
    pub active_upstream: Option<CandidateHealth>,
    pub candidates: Vec<CandidateHealth>,
    pub last_failure: Option<DiagnosticEvent>,
    pub remote_control: RemoteControlStatus,
    pub codex_path: Option<String>,
    pub uptime_secs: u64,
    /// A short, user-facing explanation and next action. Added fields remain
    /// backwards compatible for JSON-RPC consumers that ignore unknown keys.
    pub guidance: UserGuidance,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserGuidance {
    pub title: String,
    pub detail: String,
    pub action_label: Option<String>,
    /// One of `refresh`, `resume_protection`, `open_codex`, or `wait`.
    pub action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    pub timestamp: DateTime<Utc>,
    pub class: FailureClass,
    pub summary: String,
    pub source: String,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteControlStatus {
    pub enabled: bool,
    pub supported: bool,
    pub online: bool,
    pub detail: Option<String>,
    pub retry_after: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(
        source: CandidateSource,
        state: HealthState,
        latency: Option<u64>,
    ) -> CandidateHealth {
        let candidate = UpstreamCandidate::from_url(
            Url::parse("http://127.0.0.1:7890").unwrap(),
            source,
            "test",
        )
        .unwrap();
        CandidateHealth {
            candidate,
            state,
            latency_ms: latency,
            checked_at: Utc::now(),
            consecutive_failures: 0,
            failure: None,
            detail: None,
        }
    }

    #[test]
    fn healthy_candidate_beats_manual_but_down_candidate() {
        let mut values = vec![
            health(CandidateSource::Manual, HealthState::Down, Some(1)),
            health(
                CandidateSource::KnownLoopback,
                HealthState::Healthy,
                Some(20),
            ),
        ];
        sort_candidates(&mut values);
        assert_eq!(values[0].state, HealthState::Healthy);
    }

    #[test]
    fn source_priority_breaks_equal_health_ties() {
        let mut values = vec![
            health(CandidateSource::Environment, HealthState::Healthy, Some(1)),
            health(CandidateSource::SystemPac, HealthState::Healthy, Some(20)),
        ];
        sort_candidates(&mut values);
        assert_eq!(values[0].candidate.source, CandidateSource::SystemPac);
    }

    #[test]
    fn credentials_are_redacted() {
        let candidate = UpstreamCandidate::from_url(
            Url::parse("http://alice:secret@127.0.0.1:7890").unwrap(),
            CandidateSource::Manual,
            "manual",
        )
        .unwrap();
        let text = format!("{candidate:?}");
        assert!(!text.contains("alice"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn serialized_candidates_never_expose_proxy_credentials() {
        let candidate = UpstreamCandidate::from_url(
            Url::parse("http://alice:top-secret@127.0.0.1:7890").unwrap(),
            CandidateSource::Manual,
            "manual",
        )
        .unwrap();
        let encoded = serde_json::to_string(&candidate).unwrap();
        assert!(!encoded.contains("alice"));
        assert!(!encoded.contains("top-secret"));
        assert!(encoded.contains("redacted"));
    }
}
