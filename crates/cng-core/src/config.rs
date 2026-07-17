use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_LISTEN_PORT: u16 = 17_890;
pub const APP_DIR_NAME: &str = "Codex Network Guard";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardConfig {
    pub listen: SocketAddr,
    pub mode: GuardMode,
    pub health_interval_secs: u64,
    pub connect_timeout_ms: u64,
    pub direct_fallback: bool,
    pub remote_control_keepalive: bool,
    pub paused: bool,
    pub manual_upstream: Option<String>,
    pub manual_upstream_keychain: bool,
    pub codex_path: Option<PathBuf>,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_LISTEN_PORT),
            mode: GuardMode::Auto,
            health_interval_secs: 5,
            connect_timeout_ms: 2_500,
            direct_fallback: false,
            remote_control_keepalive: true,
            paused: false,
            manual_upstream: None,
            manual_upstream_keychain: false,
            codex_path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GuardMode {
    #[default]
    Auto,
    Manual,
}

impl GuardConfig {
    pub fn load_or_create() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("read configuration at {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("parse configuration at {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        write_private(&path, toml::to_string_pretty(self)?.as_bytes())
    }
}

pub fn app_support_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = dirs::data_local_dir();
    #[cfg(not(target_os = "windows"))]
    let base = dirs::data_dir().context("cannot determine user application data directory")?;
    #[cfg(target_os = "windows")]
    let base = base.context("cannot determine local application data directory")?;
    Ok(base.join(APP_DIR_NAME))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_support_dir()?.join("config.toml"))
}

pub fn rpc_socket_path() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        Ok(PathBuf::from(r"\\.\pipe\codex-network-guard-v1"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(app_support_dir()?.join("cng.sock"))
    }
}

pub fn log_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        Ok(app_support_dir()?.join("logs"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(dirs::home_dir()
            .context("cannot determine home directory")?
            .join("Library/Logs/Codex Network Guard"))
    }
}

pub fn installed_bin_dir() -> Result<PathBuf> {
    Ok(app_support_dir()?.join("bin"))
}

pub fn write_private(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)
        .with_context(|| format!("write temporary file {}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)
        .with_context(|| format!("replace configuration {}", path.display()))?;
    Ok(())
}

pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let config = GuardConfig::default();
        assert!(config.listen.ip().is_loopback());
        assert!(!config.direct_fallback);
        assert_eq!(config.health_interval_secs, 5);
    }

    #[test]
    fn older_partial_config_receives_new_defaults() {
        let config: GuardConfig = toml::from_str("health_interval_secs = 9").unwrap();
        assert_eq!(config.health_interval_secs, 9);
        assert!(!config.direct_fallback);
        assert_eq!(config.listen.port(), DEFAULT_LISTEN_PORT);
    }
}
