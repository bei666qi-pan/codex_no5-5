use anyhow::{Context, Result};

#[tokio::main]
async fn main() {
    match run().await {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("Codex Network Guard wrapper: {error:#}");
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<i32> {
    let config = cng_core::GuardConfig::load_or_create()?;
    let real = cng_core::codex::find_real_codex(Some(&config))
        .context("could not find the real Codex executable")?;
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let status = cng_core::codex::run_wrapped(&real, args).await?;
    Ok(status.code().unwrap_or(1))
}
