use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub const RPC_API_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub version: u32,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub version: u32,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

#[async_trait]
pub trait RpcHandler: Send + Sync + 'static {
    async fn handle(&self, method: &str, params: Value) -> Result<Value>;
}

#[cfg(unix)]
pub async fn run_server(path: &Path, handler: Arc<dyn RpcHandler>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("remove stale RPC socket {}", path.display()))?;
    }
    if let Some(parent) = path.parent() {
        crate::config::ensure_private_dir(parent)?;
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("bind RPC socket {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;

    loop {
        let (stream, _) = listener.accept().await?;
        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            loop {
                line.clear();
                let count = match reader.read_line(&mut line).await {
                    Ok(count) => count,
                    Err(_) => return,
                };
                if count == 0 {
                    return;
                }
                let response = process_line(&line, handler.as_ref()).await;
                let Ok(mut encoded) = serde_json::to_vec(&response) else {
                    return;
                };
                encoded.push(b'\n');
                if writer.write_all(&encoded).await.is_err() {
                    return;
                }
            }
        });
    }
}

#[cfg(not(unix))]
pub async fn run_server(_path: &Path, _handler: Arc<dyn RpcHandler>) -> Result<()> {
    anyhow::bail!("local RPC transport is not implemented on this platform")
}

async fn process_line(line: &str, handler: &dyn RpcHandler) -> RpcResponse {
    let request = match serde_json::from_str::<RpcRequest>(line) {
        Ok(request) => request,
        Err(error) => {
            return RpcResponse {
                version: RPC_API_VERSION,
                id: 0,
                result: None,
                error: Some(RpcError {
                    code: "invalid_request".to_string(),
                    message: error.to_string(),
                }),
            };
        }
    };
    if request.version != RPC_API_VERSION {
        return RpcResponse {
            version: RPC_API_VERSION,
            id: request.id,
            result: None,
            error: Some(RpcError {
                code: "unsupported_version".to_string(),
                message: format!(
                    "client requested RPC v{}, daemon supports v{}",
                    request.version, RPC_API_VERSION
                ),
            }),
        };
    }
    match handler.handle(&request.method, request.params).await {
        Ok(value) => RpcResponse {
            version: RPC_API_VERSION,
            id: request.id,
            result: Some(value),
            error: None,
        },
        Err(error) => RpcResponse {
            version: RPC_API_VERSION,
            id: request.id,
            result: None,
            error: Some(RpcError {
                code: "request_failed".to_string(),
                message: crate::proxy::redact_error(&error.to_string()),
            }),
        },
    }
}

#[cfg(unix)]
pub async fn call(path: &Path, method: &str, params: Value) -> Result<Value> {
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(path)
        .await
        .with_context(|| format!("connect to daemon at {}", path.display()))?;
    let (reader, mut writer) = stream.into_split();
    let request = RpcRequest {
        version: RPC_API_VERSION,
        id: 1,
        method: method.to_string(),
        params,
    };
    let mut encoded = serde_json::to_vec(&request)?;
    encoded.push(b'\n');
    writer.write_all(&encoded).await?;
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let response: RpcResponse = serde_json::from_str(&line).context("parse daemon response")?;
    if let Some(error) = response.error {
        anyhow::bail!("{}: {}", error.code, error.message);
    }
    response
        .result
        .context("daemon response did not include a result")
}

#[cfg(not(unix))]
pub async fn call(_path: &Path, _method: &str, _params: Value) -> Result<Value> {
    anyhow::bail!("local RPC transport is not implemented on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    #[async_trait]
    impl RpcHandler for Echo {
        async fn handle(&self, method: &str, params: Value) -> Result<Value> {
            Ok(serde_json::json!({ "method": method, "params": params }))
        }
    }

    #[tokio::test]
    async fn rejects_unknown_protocol_versions() {
        let request = serde_json::json!({
            "version": 99,
            "id": 7,
            "method": "status",
            "params": null
        });
        let response = process_line(&request.to_string(), &Echo).await;
        assert_eq!(response.id, 7);
        assert_eq!(response.error.unwrap().code, "unsupported_version");
    }
}
