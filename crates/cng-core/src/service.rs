use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::config::{app_support_dir, installed_bin_dir, log_dir};

pub const SERVICE_LABEL: &str = "dev.codex-network-guard.daemon";
pub const LEGACY_SERVICE_LABEL: &str = "com.openai.codex-proxy-guard";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub legacy_guard_detected: bool,
    pub launch_agent: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct InstallState {
    previous_codex_cli_path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MigrationReport {
    pub backup_directory: String,
    pub legacy_service_disabled: bool,
    pub legacy_files_deleted: bool,
}

const TERMINAL_BLOCK_START: &str = "# >>> codex-network-guard >>>";
const TERMINAL_BLOCK_END: &str = "# <<< codex-network-guard <<<";

#[cfg(target_os = "macos")]
pub fn launch_agent_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine home directory")?
        .join(format!("Library/LaunchAgents/{SERVICE_LABEL}.plist")))
}

#[cfg(not(target_os = "macos"))]
pub fn launch_agent_path() -> Result<PathBuf> {
    anyhow::bail!("service management is currently implemented only on macOS")
}

#[cfg(target_os = "macos")]
pub async fn status() -> Result<ServiceStatus> {
    let path = launch_agent_path()?;
    let uid = unsafe { libc::geteuid() };
    let output = Command::new("/bin/launchctl")
        .args(["print", &format!("gui/{uid}/{SERVICE_LABEL}")])
        .stdin(Stdio::null())
        .output()
        .await?;
    let legacy = dirs::home_dir()
        .map(|home| {
            home.join("Library/LaunchAgents/com.openai.codex-proxy-guard.plist")
                .exists()
        })
        .unwrap_or(false);
    Ok(ServiceStatus {
        installed: path.exists(),
        running: output.status.success(),
        legacy_guard_detected: legacy,
        launch_agent: path.display().to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub async fn status() -> Result<ServiceStatus> {
    anyhow::bail!("service management is currently implemented only on macOS")
}

#[cfg(target_os = "macos")]
pub async fn install() -> Result<ServiceStatus> {
    let executable = std::env::current_exe()?;
    let source_dir = executable
        .parent()
        .context("cannot locate CNG installation directory")?;
    let cli_source = source_dir.join("cng");
    let daemon_source = source_dir.join("cngd");
    let wrapper_source = source_dir.join("cng-codex");
    anyhow::ensure!(cli_source.is_file(), "cng must be next to the installer");
    anyhow::ensure!(daemon_source.is_file(), "cngd must be next to cng");
    anyhow::ensure!(wrapper_source.is_file(), "cng-codex must be next to cng");

    let bin_dir = installed_bin_dir()?;
    crate::config::ensure_private_dir(&bin_dir)?;
    let daemon = bin_dir.join("cngd");
    let wrapper = bin_dir.join("cng-codex");
    let cli = bin_dir.join("cng");
    copy_executable(&daemon_source, &daemon)?;
    copy_executable(&wrapper_source, &wrapper)?;
    copy_executable(&cli_source, &cli)?;
    let terminal_wrapper = bin_dir.join("codex");
    if terminal_wrapper.exists() {
        fs::remove_file(&terminal_wrapper)?;
    }
    std::os::unix::fs::symlink(&wrapper, &terminal_wrapper)?;
    let logs = log_dir()?;
    fs::create_dir_all(&logs)?;
    let plist = launch_agent_xml(&daemon, &wrapper, &logs);
    let path = launch_agent_path()?;
    crate::config::write_private(&path, plist.as_bytes())?;

    let uid = unsafe { libc::geteuid() };
    let domain = format!("gui/{uid}");
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &domain, path.to_string_lossy().as_ref()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    let status = Command::new("/bin/launchctl")
        .args(["bootstrap", &domain, path.to_string_lossy().as_ref()])
        .status()
        .await?;
    anyhow::ensure!(status.success(), "launchctl bootstrap failed");
    save_previous_environment(&wrapper).await?;
    let status = Command::new("/bin/launchctl")
        .args([
            "setenv",
            "CODEX_CLI_PATH",
            wrapper.to_string_lossy().as_ref(),
        ])
        .status()
        .await?;
    anyhow::ensure!(status.success(), "could not set CODEX_CLI_PATH");
    for _ in 0..24 {
        let service = self::status().await?;
        let rpc_ready = crate::rpc::call(
            &crate::config::rpc_socket_path()?,
            "status",
            serde_json::Value::Null,
        )
        .await
        .is_ok();
        if service.running && rpc_ready {
            return Ok(service);
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let _ = uninstall().await;
    anyhow::bail!("daemon did not become healthy within six seconds; installation was rolled back")
}

#[cfg(not(target_os = "macos"))]
pub async fn install() -> Result<ServiceStatus> {
    anyhow::bail!("service management is currently implemented only on macOS")
}

#[cfg(target_os = "macos")]
pub async fn uninstall() -> Result<ServiceStatus> {
    let path = launch_agent_path()?;
    let uid = unsafe { libc::geteuid() };
    let domain = format!("gui/{uid}");
    if path.exists() {
        let _ = Command::new("/bin/launchctl")
            .args(["bootout", &domain, path.to_string_lossy().as_ref()])
            .status()
            .await;
        fs::remove_file(&path).context("remove LaunchAgent")?;
    }
    let expected_wrapper = installed_bin_dir()?.join("cng-codex");
    let current = Command::new("/bin/launchctl")
        .args(["getenv", "CODEX_CLI_PATH"])
        .output()
        .await?;
    if String::from_utf8_lossy(&current.stdout).trim() == expected_wrapper.to_string_lossy() {
        restore_previous_environment().await?;
    } else {
        let state = app_support_dir()?.join("install-state.json");
        if state.exists() {
            fs::remove_file(state)?;
        }
    }
    let _ = disable_terminal_path();
    let bin_dir = installed_bin_dir()?;
    if bin_dir.exists() {
        fs::remove_dir_all(bin_dir).context("remove installed CNG binaries")?;
    }
    self::status().await
}

#[cfg(not(target_os = "macos"))]
pub async fn uninstall() -> Result<ServiceStatus> {
    anyhow::bail!("service management is currently implemented only on macOS")
}

#[cfg(target_os = "macos")]
pub async fn restart() -> Result<ServiceStatus> {
    let uid = unsafe { libc::geteuid() };
    let target = format!("gui/{uid}/{SERVICE_LABEL}");
    let status = Command::new("/bin/launchctl")
        .args(["kickstart", "-k", &target])
        .status()
        .await?;
    anyhow::ensure!(status.success(), "launchctl kickstart failed");
    self::status().await
}

#[cfg(not(target_os = "macos"))]
pub async fn restart() -> Result<ServiceStatus> {
    anyhow::bail!("service management is currently implemented only on macOS")
}

#[cfg(target_os = "macos")]
fn copy_executable(source: &Path, destination: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if source.canonicalize().ok() == destination.canonicalize().ok() && destination.is_file() {
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755))?;
        return Ok(());
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "copy executable from {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_agent_xml(daemon: &Path, wrapper: &Path, logs: &Path) -> String {
    let escape = |value: &Path| xml_escape(&value.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{SERVICE_LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string></array>
  <key>EnvironmentVariables</key>
  <dict><key>CODEX_CLI_PATH</key><string>{}</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>StandardOutPath</key><string>{}/daemon.stdout.log</string>
  <key>StandardErrorPath</key><string>{}/daemon.stderr.log</string>
</dict>
</plist>
"#,
        escape(daemon),
        escape(wrapper),
        escape(logs),
        escape(logs)
    )
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn legacy_guard_backup_dir() -> Result<PathBuf> {
    Ok(app_support_dir()?.join("migration-backup"))
}

#[cfg(target_os = "macos")]
async fn save_previous_environment(expected_wrapper: &Path) -> Result<()> {
    let path = app_support_dir()?.join("install-state.json");
    if path.exists() {
        return Ok(());
    }
    let output = Command::new("/bin/launchctl")
        .args(["getenv", "CODEX_CLI_PATH"])
        .output()
        .await?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let state = InstallState {
        previous_codex_cli_path: (!value.is_empty() && value != expected_wrapper.to_string_lossy())
            .then_some(value),
    };
    crate::config::write_private(&path, &serde_json::to_vec_pretty(&state)?)
}

#[cfg(target_os = "macos")]
async fn restore_previous_environment() -> Result<()> {
    let state_path = app_support_dir()?.join("install-state.json");
    let state = fs::read(&state_path)
        .ok()
        .and_then(|value| serde_json::from_slice::<InstallState>(&value).ok())
        .unwrap_or_default();
    let mut command = Command::new("/bin/launchctl");
    if let Some(previous) = state.previous_codex_cli_path {
        command.args(["setenv", "CODEX_CLI_PATH", &previous]);
    } else {
        command.args(["unsetenv", "CODEX_CLI_PATH"]);
    }
    let status = command.status().await?;
    anyhow::ensure!(status.success(), "could not restore CODEX_CLI_PATH");
    if state_path.exists() {
        fs::remove_file(state_path)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub async fn migrate_legacy() -> Result<MigrationReport> {
    let current = status().await?;
    anyhow::ensure!(
        current.installed && current.running,
        "install and verify Codex Network Guard before migrating the legacy guard"
    );
    anyhow::ensure!(
        current.legacy_guard_detected,
        "no legacy guard was detected"
    );
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let legacy_plist = home.join(format!("Library/LaunchAgents/{LEGACY_SERVICE_LABEL}.plist"));
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let backup = legacy_guard_backup_dir()?.join(timestamp);
    crate::config::ensure_private_dir(&backup)?;
    fs::copy(&legacy_plist, backup.join("legacy-launch-agent.plist"))?;
    for candidate in [
        home.join(".local/bin/codex-proxy-guard"),
        home.join(".local/bin/codex-proxy-guard.sh"),
    ] {
        if candidate.is_file() {
            let name = candidate
                .file_name()
                .context("legacy path has no file name")?;
            fs::copy(&candidate, backup.join(name))?;
        }
    }
    let uid = unsafe { libc::geteuid() };
    let status = Command::new("/bin/launchctl")
        .args([
            "bootout",
            &format!("gui/{uid}"),
            legacy_plist.to_string_lossy().as_ref(),
        ])
        .status()
        .await?;
    anyhow::ensure!(status.success(), "legacy LaunchAgent could not be disabled");
    Ok(MigrationReport {
        backup_directory: backup.display().to_string(),
        legacy_service_disabled: true,
        legacy_files_deleted: false,
    })
}

#[cfg(not(target_os = "macos"))]
pub async fn migrate_legacy() -> Result<MigrationReport> {
    anyhow::bail!("legacy migration is currently implemented only on macOS")
}

#[cfg(target_os = "macos")]
pub fn enable_terminal_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let profile = home.join(".zprofile");
    let original = fs::read_to_string(&profile).unwrap_or_default();
    if original.contains(TERMINAL_BLOCK_START) {
        return Ok(profile);
    }
    let backup = app_support_dir()?.join("shell-backup");
    crate::config::ensure_private_dir(&backup)?;
    if profile.exists() && !backup.join("zprofile").exists() {
        fs::copy(&profile, backup.join("zprofile"))?;
    }
    let bin = installed_bin_dir()?;
    let block = format!(
        "{TERMINAL_BLOCK_START}\nexport PATH=\"{}:$PATH\"\n{TERMINAL_BLOCK_END}\n",
        shell_double_quote(&bin.to_string_lossy())
    );
    let separator = if !original.is_empty() && !original.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    write_preserving_permissions(&profile, format!("{original}{separator}{block}").as_bytes())?;
    Ok(profile)
}

#[cfg(not(target_os = "macos"))]
pub fn enable_terminal_path() -> Result<PathBuf> {
    anyhow::bail!("terminal PATH integration is currently implemented only on macOS")
}

#[cfg(target_os = "macos")]
pub fn disable_terminal_path() -> Result<PathBuf> {
    let profile = dirs::home_dir()
        .context("cannot determine home directory")?
        .join(".zprofile");
    if !profile.exists() {
        return Ok(profile);
    }
    let original = fs::read_to_string(&profile)?;
    let cleaned = remove_managed_block(&original);
    write_preserving_permissions(&profile, cleaned.as_bytes())?;
    Ok(profile)
}

#[cfg(not(target_os = "macos"))]
pub fn disable_terminal_path() -> Result<PathBuf> {
    anyhow::bail!("terminal PATH integration is currently implemented only on macOS")
}

fn remove_managed_block(value: &str) -> String {
    let Some(start) = value.find(TERMINAL_BLOCK_START) else {
        return value.to_string();
    };
    let Some(relative_end) = value[start..].find(TERMINAL_BLOCK_END) else {
        return value.to_string();
    };
    let mut end = start + relative_end + TERMINAL_BLOCK_END.len();
    if value[end..].starts_with('\n') {
        end += 1;
    }
    let mut output = format!("{}{}", &value[..start], &value[end..]);
    while output.ends_with("\n\n") {
        output.pop();
    }
    output
}

#[cfg(target_os = "macos")]
fn write_preserving_permissions(path: &Path, content: &[u8]) -> Result<()> {
    let permissions = fs::metadata(path).ok().map(|value| value.permissions());
    fs::write(path, content)?;
    if let Some(permissions) = permissions {
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn shell_double_quote(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn launch_agent_escapes_paths() {
        let xml = launch_agent_xml(
            Path::new("/tmp/a&b/cngd"),
            Path::new("/tmp/wrapper"),
            Path::new("/tmp/logs"),
        );
        assert!(xml.contains("a&amp;b"));
        assert!(xml.contains(SERVICE_LABEL));
    }

    #[test]
    fn removes_only_our_terminal_block() {
        let value =
            format!("before\n{TERMINAL_BLOCK_START}\nexport PATH=x\n{TERMINAL_BLOCK_END}\nafter\n");
        assert_eq!(remove_managed_block(&value), "before\nafter\n");
    }

    #[test]
    fn escapes_shell_metacharacters() {
        assert_eq!(shell_double_quote("a$b`c\"d"), "a\\$b\\`c\\\"d");
    }
}
