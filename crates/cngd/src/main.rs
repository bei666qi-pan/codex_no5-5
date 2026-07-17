use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use async_trait::async_trait;
use cng_core::codex;
use cng_core::config::{GuardConfig, rpc_socket_path};
use cng_core::diagnostics::{DoctorReport, latest_codex_diagnostic};
use cng_core::discovery;
use cng_core::model::{GuardStatus, GuardStatusKind, HealthState, RemoteControlStatus};
use cng_core::proxy::{ProxyRuntime, refresh_health, run_proxy};
use cng_core::rpc::{RPC_API_VERSION, RpcHandler};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::process::Child;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

struct AppState {
    config: RwLock<GuardConfig>,
    runtime: ProxyRuntime,
    started: Instant,
    remote: RwLock<RemoteControlStatus>,
    remote_child: Mutex<Option<Child>>,
}

impl AppState {
    async fn status(&self) -> GuardStatus {
        let config = self.config.read().await.clone();
        let candidates = self.runtime.candidates().await;
        let active = candidates
            .iter()
            .find(|value| value.state == HealthState::Healthy)
            .cloned()
            .or_else(|| {
                candidates
                    .iter()
                    .find(|value| value.state == HealthState::Degraded)
                    .cloned()
            });
        let last_failure = latest_codex_diagnostic();
        let status = if config.paused {
            GuardStatusKind::Paused
        } else if active
            .as_ref()
            .is_some_and(|value| value.state == HealthState::Healthy)
        {
            GuardStatusKind::Protected
        } else if active.is_some() {
            GuardStatusKind::Degraded
        } else if last_failure.as_ref().is_some_and(|event| {
            matches!(
                event.class,
                cng_core::FailureClass::AppServerCrash
                    | cng_core::FailureClass::ToolState
                    | cng_core::FailureClass::Authentication
                    | cng_core::FailureClass::RateLimit
                    | cng_core::FailureClass::Server
            )
        }) {
            GuardStatusKind::NonNetworkFailure
        } else {
            GuardStatusKind::VpnUnavailable
        };
        GuardStatus {
            api_version: RPC_API_VERSION,
            status,
            paused: config.paused,
            listen: config.listen.to_string(),
            active_upstream: active,
            candidates,
            last_failure,
            remote_control: self.remote.read().await.clone(),
            codex_path: codex::find_real_codex(Some(&config))
                .map(|path| path.display().to_string()),
            uptime_secs: self.started.elapsed().as_secs(),
        }
    }

    async fn save_config(&self, config: GuardConfig) -> Result<()> {
        config.save()?;
        self.runtime.set_paused(config.paused);
        self.runtime.set_direct_fallback(config.direct_fallback);
        self.runtime
            .set_connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .await;
        *self.config.write().await = config;
        Ok(())
    }
}

#[async_trait]
impl RpcHandler for AppState {
    async fn handle(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "status" | "upstream.list" => Ok(serde_json::to_value(self.status().await)?),
            "refresh" => {
                refresh_once(self).await?;
                Ok(serde_json::to_value(self.status().await)?)
            }
            "pause" => {
                let paused = params
                    .get("paused")
                    .and_then(Value::as_bool)
                    .context("pause requires a boolean 'paused' parameter")?;
                let mut config = self.config.read().await.clone();
                config.paused = paused;
                self.save_config(config).await?;
                Ok(serde_json::to_value(self.status().await)?)
            }
            "upstream.auto" => {
                let mut config = self.config.read().await.clone();
                config.mode = cng_core::config::GuardMode::Auto;
                config.manual_upstream = None;
                config.manual_upstream_keychain = false;
                self.save_config(config).await?;
                refresh_once(self).await?;
                Ok(serde_json::to_value(self.status().await)?)
            }
            "upstream.set" => {
                let value = params
                    .get("url")
                    .and_then(Value::as_str)
                    .context("upstream.set requires a URL")?;
                let stored_in_keychain = discovery::save_manual_upstream(value).await?;
                let mut config = self.config.read().await.clone();
                config.mode = cng_core::config::GuardMode::Manual;
                config.manual_upstream_keychain = stored_in_keychain;
                config.manual_upstream = (!stored_in_keychain).then(|| value.to_string());
                self.save_config(config).await?;
                refresh_once(self).await?;
                Ok(serde_json::to_value(self.status().await)?)
            }
            "doctor" => {
                let status = self.status().await;
                let codex_report = match status.codex_path.as_deref() {
                    Some(path) => codex::doctor(Path::new(path)).await.ok(),
                    None => None,
                };
                let findings = status.last_failure.clone().into_iter().collect();
                Ok(serde_json::to_value(DoctorReport {
                    generated_at: chrono::Utc::now(),
                    guard: status,
                    codex: codex_report,
                    findings,
                })?)
            }
            "remote.start" | "remote.stop" | "remote.pair" => {
                let action = method.trim_start_matches("remote.");
                let mut config = self.config.read().await.clone();
                let real = codex::find_real_codex(Some(&config)).context("Codex was not found")?;
                anyhow::ensure!(
                    codex::supports_remote_control(&real).await,
                    "this Codex version does not provide remote-control"
                );
                match action {
                    "start" => {
                        config.remote_control_keepalive = true;
                        self.save_config(config).await?;
                        ensure_remote_started(self, &real).await?;
                        Ok(serde_json::to_value(self.remote.read().await.clone())?)
                    }
                    "stop" => {
                        config.remote_control_keepalive = false;
                        self.save_config(config).await?;
                        if let Some(mut child) = self.remote_child.lock().await.take() {
                            let _ = child.kill().await;
                        }
                        let _ = tokio::time::timeout(
                            Duration::from_secs(10),
                            codex::remote_control(&real, "stop"),
                        )
                        .await;
                        let mut remote = self.remote.write().await;
                        remote.supported = true;
                        remote.enabled = false;
                        remote.online = false;
                        remote.retry_after = None;
                        remote.detail = Some("remote-control stopped".into());
                        Ok(serde_json::to_value(remote.clone())?)
                    }
                    "pair" => {
                        let value = codex::remote_control(&real, "pair").await?;
                        let mut remote = self.remote.write().await;
                        remote.supported = true;
                        remote.detail = Some("pairing flow completed".into());
                        Ok(value)
                    }
                    _ => unreachable!(),
                }
            }
            _ => anyhow::bail!("unknown RPC method: {method}"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = init_logging()?;
    let config = GuardConfig::load_or_create()?;
    anyhow::ensure!(
        config.listen.ip().is_loopback(),
        "refusing to bind a non-loopback proxy address"
    );
    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("bind relay at {}", config.listen))?;
    let runtime = ProxyRuntime::new(&config);
    let state = Arc::new(AppState {
        config: RwLock::new(config),
        runtime: runtime.clone(),
        started: Instant::now(),
        remote: RwLock::new(RemoteControlStatus::default()),
        remote_child: Mutex::new(None),
    });

    refresh_once(state.as_ref()).await?;
    tokio::spawn(run_health_loop(Arc::clone(&state)));
    tokio::spawn(run_remote_keepalive(Arc::clone(&state)));

    let socket = rpc_socket_path()?;
    info!(socket = %socket.display(), "starting local RPC server");
    let proxy_task = tokio::spawn(run_proxy(listener, runtime));
    let rpc_state = Arc::clone(&state) as Arc<dyn RpcHandler>;
    let rpc_task = tokio::spawn(async move { cng_core::rpc::run_server(&socket, rpc_state).await });
    tokio::select! {
        result = proxy_task => result??,
        result = rpc_task => result??,
    }
    Ok(())
}

async fn refresh_once(state: &AppState) -> Result<()> {
    let config = state.config.read().await.clone();
    let discovered = discovery::discover(&config).await?;
    let previous = state.runtime.candidates().await;
    let health = refresh_health(
        discovered,
        &previous,
        Duration::from_millis(config.connect_timeout_ms),
    )
    .await;
    let healthy = health
        .iter()
        .filter(|value| value.state == HealthState::Healthy)
        .count();
    info!(
        candidates = health.len(),
        healthy, "proxy discovery completed"
    );
    state.runtime.replace_candidates(health).await;
    Ok(())
}

async fn run_health_loop(state: Arc<AppState>) {
    loop {
        let interval = state.config.read().await.health_interval_secs.max(1);
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if let Err(error) = refresh_once(state.as_ref()).await {
            warn!(error = %error, "health refresh failed");
        }
    }
}

async fn run_remote_keepalive(state: Arc<AppState>) {
    let mut backoff = Duration::from_secs(1);
    let mut last_started: Option<Instant> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let config = state.config.read().await.clone();
        if !config.remote_control_keepalive {
            continue;
        }
        let child_state = {
            let mut guard = state.remote_child.lock().await;
            match guard.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(None) => Some(Ok(())),
                    Ok(Some(status)) => {
                        *guard = None;
                        Some(Err(format!("remote-control exited with {status}")))
                    }
                    Err(error) => {
                        *guard = None;
                        Some(Err(format!("could not inspect remote-control: {error}")))
                    }
                },
                None => None,
            }
        };
        if matches!(child_state, Some(Ok(()))) {
            if last_started.is_some_and(|started| started.elapsed() >= Duration::from_secs(30)) {
                backoff = Duration::from_secs(1);
            }
            continue;
        }
        if let Some(Err(detail)) = child_state {
            let mut remote = state.remote.write().await;
            remote.online = false;
            remote.detail = Some(detail);
            remote.retry_after = Some(backoff);
            drop(remote);
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(300));
        }
        let Some(real) = codex::find_real_codex(Some(&config)) else {
            continue;
        };
        if !codex::supports_remote_control(&real).await {
            let mut remote = state.remote.write().await;
            remote.supported = false;
            remote.detail = Some("remote-control is unavailable in this Codex version".into());
            continue;
        }
        match ensure_remote_started(state.as_ref(), &real).await {
            Ok(()) => {
                last_started = Some(Instant::now());
            }
            Err(error) => {
                error!(error = %error, "remote-control keepalive failed");
                let mut remote = state.remote.write().await;
                remote.enabled = true;
                remote.supported = true;
                remote.online = false;
                remote.detail = Some(cng_core::proxy::redact_error(&error.to_string()));
                remote.retry_after = Some(backoff);
                drop(remote);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(300));
            }
        }
    }
}

async fn ensure_remote_started(state: &AppState, real: &Path) -> Result<()> {
    let mut child = state.remote_child.lock().await;
    if child
        .as_mut()
        .is_some_and(|value| value.try_wait().ok() == Some(None))
    {
        return Ok(());
    }
    *child = Some(codex::spawn_remote_control(real).await?);
    drop(child);
    let mut remote = state.remote.write().await;
    remote.enabled = true;
    remote.supported = true;
    remote.online = true;
    remote.detail = Some("remote-control is online".into());
    remote.retry_after = None;
    Ok(())
}

fn init_logging() -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let directory = cng_core::config::log_dir()?;
    fs::create_dir_all(&directory)?;
    prune_logs(&directory);
    let appender = tracing_appender::rolling::daily(directory, "daemon.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("cng=info")),
        )
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Ok(guard)
}

fn prune_logs(directory: &Path) {
    const MAX_LOG_BYTES: u64 = 20 * 1024 * 1024;
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(7 * 24 * 60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut retained = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let old = modified < cutoff;
        if old {
            let _ = fs::remove_file(path);
        } else {
            retained.push((path, modified, metadata.len()));
        }
    }
    retained.sort_by_key(|(_, modified, _)| *modified);
    let mut total = retained.iter().map(|(_, _, bytes)| bytes).sum::<u64>();
    for (path, _, bytes) in retained {
        if total <= MAX_LOG_BYTES {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(bytes);
        }
    }
}
