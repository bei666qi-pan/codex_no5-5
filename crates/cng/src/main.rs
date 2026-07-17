use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use cng_core::model::{GuardStatus, GuardStatusKind};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "cng", version, about = "Codex Network Guard")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show current guard, proxy, and remote-control status.
    Status(OutputArgs),
    /// Refresh all proxy candidates immediately.
    Refresh(OutputArgs),
    /// Run redacted guard and Codex connectivity diagnostics.
    Doctor(DoctorArgs),
    /// Inspect or select the upstream proxy.
    Upstream {
        #[command(subcommand)]
        command: UpstreamCommand,
    },
    /// Run the real Codex CLI through the stable relay.
    Codex(CodexArgs),
    /// Manage official Codex mobile remote control.
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
    /// Install, restart, or remove the per-user daemon.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Pause or resume routing without uninstalling.
    Pause {
        #[arg(value_parser = ["on", "off"])]
        state: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args)]
struct OutputArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct DoctorArgs {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    export: Option<PathBuf>,
}

#[derive(Args)]
struct CodexArgs {
    #[arg(last = true, allow_hyphen_values = true)]
    args: Vec<OsString>,
}

#[derive(Subcommand)]
enum UpstreamCommand {
    List(OutputArgs),
    Set { url: String },
    Auto,
}

#[derive(Subcommand)]
enum RemoteCommand {
    Start,
    Stop,
    Pair,
}

#[derive(Subcommand, Clone, Copy)]
enum ServiceCommand {
    Status,
    Install,
    Restart,
    Uninstall,
    /// Back up and disable a detected codex-proxy-guard after CNG is healthy.
    MigrateLegacy,
    /// Add the reversible CNG PATH block to ~/.zprofile.
    TerminalEnable,
    /// Remove only the CNG PATH block from ~/.zprofile.
    TerminalDisable,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("cng: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status(args) => {
            let value = rpc("status", Value::Null).await?;
            print_status(value, args.json)?;
        }
        Command::Refresh(args) => {
            let value = rpc("refresh", Value::Null).await?;
            print_status(value, args.json)?;
        }
        Command::Doctor(args) => {
            let value = rpc("doctor", Value::Null).await?;
            if let Some(ref path) = args.export {
                let encoded = serde_json::to_vec_pretty(&value)?;
                fs::write(path, encoded)
                    .with_context(|| format!("write diagnostic report to {}", path.display()))?;
                println!("Redacted diagnostic report: {}", path.display());
            }
            if args.json || args.export.is_none() {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
        }
        Command::Upstream { command } => match command {
            UpstreamCommand::List(args) => {
                let value = rpc("upstream.list", Value::Null).await?;
                print_status(value, args.json)?;
            }
            UpstreamCommand::Set { url } => {
                let value = if url.eq_ignore_ascii_case("auto") {
                    rpc("upstream.auto", Value::Null).await?
                } else {
                    rpc("upstream.set", json!({ "url": url })).await?
                };
                print_status(value, false)?;
            }
            UpstreamCommand::Auto => {
                let value = rpc("upstream.auto", Value::Null).await?;
                print_status(value, false)?;
            }
        },
        Command::Codex(args) => {
            let config = cng_core::GuardConfig::load_or_create()?;
            let real = cng_core::codex::find_real_codex(Some(&config))
                .context("could not find an installed Codex CLI")?;
            let status = cng_core::codex::run_wrapped(&real, args.args).await?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Command::Remote { command } => {
            let method = match command {
                RemoteCommand::Start => "remote.start",
                RemoteCommand::Stop => "remote.stop",
                RemoteCommand::Pair => "remote.pair",
            };
            let value = rpc(method, Value::Null).await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        Command::Service { command } => {
            if matches!(command, ServiceCommand::MigrateLegacy) {
                let report = cng_core::service::migrate_legacy().await?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(());
            }
            if matches!(command, ServiceCommand::TerminalEnable) {
                let path = cng_core::service::enable_terminal_path()?;
                println!("Terminal integration enabled in {}", path.display());
                return Ok(());
            }
            if matches!(command, ServiceCommand::TerminalDisable) {
                let path = cng_core::service::disable_terminal_path()?;
                println!("Terminal integration removed from {}", path.display());
                return Ok(());
            }
            let status = match command {
                ServiceCommand::Status => cng_core::service::status().await?,
                ServiceCommand::Install => cng_core::service::install().await?,
                ServiceCommand::Restart => cng_core::service::restart().await?,
                ServiceCommand::Uninstall => cng_core::service::uninstall().await?,
                ServiceCommand::MigrateLegacy
                | ServiceCommand::TerminalEnable
                | ServiceCommand::TerminalDisable => unreachable!(),
            };
            println!("{}", serde_json::to_string_pretty(&status)?);
            if status.legacy_guard_detected {
                eprintln!(
                    "Note: legacy com.openai.codex-proxy-guard was detected and was not modified."
                );
            }
        }
        Command::Pause { state, json } => {
            let value = rpc("pause", json!({ "paused": state == "on" })).await?;
            print_status(value, json)?;
        }
    }
    Ok(())
}

async fn rpc(method: &str, params: Value) -> Result<Value> {
    let socket = cng_core::config::rpc_socket_path()?;
    cng_core::rpc::call(&socket, method, params)
        .await
        .with_context(|| "Codex Network Guard daemon is not reachable; run `cng service install`")
}

fn print_status(value: Value, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let status: GuardStatus = serde_json::from_value(value)?;
    let headline = match status.status {
        GuardStatusKind::Protected => "Protected",
        GuardStatusKind::Degraded => "Degraded",
        GuardStatusKind::VpnUnavailable => "VPN unavailable",
        GuardStatusKind::NonNetworkFailure => "Non-network Codex failure",
        GuardStatusKind::Paused => "Paused",
    };
    println!("Codex Network Guard: {headline}");
    println!("Relay: {}", status.listen);
    if let Some(active) = status.active_upstream {
        println!(
            "Upstream: {} ({:?}, {} ms)",
            active.candidate.label,
            active.state,
            active.latency_ms.unwrap_or_default()
        );
    } else {
        println!("Upstream: none healthy");
    }
    println!(
        "Remote control: {}",
        if status.remote_control.online {
            "online"
        } else if status.remote_control.supported {
            "offline"
        } else {
            "unsupported"
        }
    );
    if let Some(failure) = status.last_failure {
        println!(
            "Latest diagnostic: {:?} — {}",
            failure.class, failure.summary
        );
    }
    println!(
        "Next step: {} — {}",
        status.guidance.title, status.guidance.detail
    );
    if let Some(action) = status.guidance.action_label {
        println!("Suggested action: {action}");
    }
    Ok(())
}
