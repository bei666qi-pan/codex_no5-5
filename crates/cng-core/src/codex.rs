use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::process::{Child, Command};

use crate::config::{DEFAULT_LISTEN_PORT, GuardConfig};

const COMMON_CODEX_PATHS: &[&str] = &[
    "/Applications/ChatGPT.app/Contents/Resources/codex",
    "/Applications/Codex.app/Contents/Resources/codex",
    "/opt/homebrew/bin/codex",
    "/usr/local/bin/codex",
];

pub fn find_real_codex(config: Option<&GuardConfig>) -> Option<PathBuf> {
    if let Some(path) = config.and_then(|value| value.codex_path.clone())
        && is_real_codex(&path)
    {
        return Some(path);
    }
    if let Some(path) = std::env::var_os("CNG_REAL_CODEX").map(PathBuf::from)
        && is_real_codex(&path)
    {
        return Some(path);
    }
    for value in COMMON_CODEX_PATHS {
        let path = PathBuf::from(value);
        if is_real_codex(&path) {
            return Some(path);
        }
    }
    find_on_path("codex")
}

fn is_real_codex(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let current = std::env::current_exe()
        .ok()
        .and_then(|value| value.canonicalize().ok());
    let candidate = path.canonicalize().ok();
    candidate.is_some() && candidate != current
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let mut seen = HashSet::new();
    for directory in std::env::split_paths(&std::env::var_os("PATH")?) {
        let candidate = directory.join(name);
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if seen.insert(canonical.clone()) && is_real_codex(&canonical) {
            return Some(canonical);
        }
    }
    None
}

pub fn apply_proxy_environment(command: &mut Command, real_codex: &Path) {
    let proxy = format!("http://127.0.0.1:{DEFAULT_LISTEN_PORT}");
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(name, &proxy);
    }
    let no_proxy = merged_no_proxy();
    command.env("NO_PROXY", &no_proxy);
    command.env("no_proxy", &no_proxy);
    command.env("CODEX_CLI_PATH", real_codex);
    command.env("CNG_WRAPPED", "1");
}

pub fn merged_no_proxy() -> String {
    let mut values = Vec::new();
    for name in ["NO_PROXY", "no_proxy"] {
        if let Ok(value) = std::env::var(name) {
            values.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            );
        }
    }
    for required in ["localhost", "127.0.0.1", "::1", ".local", ".lan"] {
        if !values.iter().any(|value| value == required) {
            values.push(required.to_string());
        }
    }
    values.sort();
    values.dedup();
    values.join(",")
}

pub async fn supports_feature(real_codex: &Path, feature: &str) -> bool {
    let mut command = Command::new(real_codex);
    command.args(["features", "list"]);
    command.env("CODEX_CLI_PATH", real_codex);
    command.stdin(Stdio::null());
    match command.output().await {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split_whitespace().next() == Some(feature)),
        _ => false,
    }
}

pub async fn wrapped_command<I, S>(
    real_codex: &Path,
    args: I,
    inherit_stdio: bool,
) -> Result<Command>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut command = Command::new(real_codex);
    apply_proxy_environment(&mut command, real_codex);
    if supports_feature(real_codex, "respect_system_proxy").await {
        command.args(["--disable", "respect_system_proxy"]);
    }
    command.args(args.into_iter().map(Into::into));
    if inherit_stdio {
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    } else {
        command.stdin(Stdio::null());
    }
    Ok(command)
}

pub async fn run_wrapped<I, S>(real_codex: &Path, args: I) -> Result<ExitStatus>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    wrapped_command(real_codex, args, true)
        .await?
        .status()
        .await
        .context("run Codex")
}

pub async fn spawn_wrapped<I, S>(real_codex: &Path, args: I) -> Result<Child>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    wrapped_command(real_codex, args, false)
        .await?
        .spawn()
        .context("start Codex")
}

pub async fn spawn_remote_control(real_codex: &Path) -> Result<Child> {
    let mut command =
        wrapped_command(real_codex, ["remote-control", "start", "--json"], false).await?;
    command.stdout(Stdio::null()).stderr(Stdio::null());
    command.spawn().context("start Codex remote-control")
}

pub async fn doctor(real_codex: &Path) -> Result<Value> {
    let output = wrapped_command(real_codex, ["doctor", "--json"], false)
        .await?
        .output()
        .await
        .context("run codex doctor")?;
    let text = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    serde_json::from_slice(text).context("parse codex doctor JSON")
}

pub async fn supports_remote_control(real_codex: &Path) -> bool {
    let mut command = Command::new(real_codex);
    command.args(["remote-control", "--help"]);
    command.env("CODEX_CLI_PATH", real_codex);
    command.stdin(Stdio::null());
    command.status().await.is_ok_and(|status| status.success())
}

pub async fn remote_control(real_codex: &Path, action: &str) -> Result<Value> {
    anyhow::ensure!(
        matches!(action, "start" | "stop" | "pair"),
        "unsupported remote-control action"
    );
    let output = wrapped_command(real_codex, ["remote-control", action, "--json"], false)
        .await?
        .output()
        .await?;
    anyhow::ensure!(
        output.status.success(),
        "remote-control {action} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    if output.stdout.is_empty() {
        return Ok(serde_json::json!({ "ok": true }));
    }
    serde_json::from_slice(&output.stdout).context("parse remote-control JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proxy_is_merged_and_deduplicated() {
        // SAFETY: this unit test is single-threaded with respect to this crate's environment use.
        unsafe { std::env::set_var("NO_PROXY", "localhost,example.com") };
        let value = merged_no_proxy();
        assert!(value.contains("example.com"));
        assert_eq!(value.matches("localhost").count(), 1);
        assert!(value.contains("127.0.0.1"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn wrapper_injects_both_proxy_cases_and_preserves_arguments() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-codex");
        fs::write(
            &executable,
            "#!/bin/sh\nif [ \"$1\" = features ]; then exit 0; fi\nprintf '%s\\n' \"$HTTP_PROXY\" \"$https_proxy\" \"$NO_PROXY\" \"$1\" \"$2\"\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let output = wrapped_command(&executable, ["--flag", "hello world"], false)
            .await
            .unwrap()
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        let lines = String::from_utf8(output.stdout).unwrap();
        assert_eq!(lines.lines().next(), Some("http://127.0.0.1:17890"));
        assert_eq!(lines.lines().nth(1), Some("http://127.0.0.1:17890"));
        assert!(lines.lines().nth(2).unwrap().contains("localhost"));
        assert!(lines.ends_with("--flag\nhello world\n"));
    }
}
