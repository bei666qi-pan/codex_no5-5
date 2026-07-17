use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cng_core::config::GuardConfig;
use cng_core::model::{CandidateHealth, CandidateSource, HealthState, UpstreamCandidate};
use cng_core::proxy::{ProxyRuntime, connect_tunnel, run_proxy};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

async fn read_header(stream: &mut TcpStream) -> Vec<u8> {
    let mut output = Vec::new();
    let mut byte = [0u8; 1];
    while output.len() < 32 * 1024 {
        stream.read_exact(&mut byte).await.unwrap();
        output.push(byte[0]);
        if output.ends_with(b"\r\n\r\n") {
            return output;
        }
    }
    panic!("header was too large");
}

async fn spawn_echo_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut reader, mut writer) = stream.split();
                let _ = tokio::io::copy(&mut reader, &mut writer).await;
            });
        }
    });
    (address, task)
}

async fn spawn_http_proxy(
    reject: bool,
) -> (
    std::net::SocketAddr,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let task_counter = Arc::clone(&accepted);
    let task = tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            task_counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let header = read_header(&mut client).await;
                if reject {
                    let _ = client
                        .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                        .await;
                    return;
                }
                let request = String::from_utf8(header).unwrap();
                let authority = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap();
                let mut upstream = TcpStream::connect(authority).await.unwrap();
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });
    (address, accepted, task)
}

async fn spawn_socks5_proxy() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut greeting = [0u8; 2];
                client.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting[0], 5);
                let mut methods = vec![0u8; greeting[1] as usize];
                client.read_exact(&mut methods).await.unwrap();
                client.write_all(&[5, 0]).await.unwrap();

                let mut request = [0u8; 4];
                client.read_exact(&mut request).await.unwrap();
                assert_eq!(&request[..3], &[5, 1, 0]);
                let host = match request[3] {
                    1 => {
                        let mut bytes = [0u8; 4];
                        client.read_exact(&mut bytes).await.unwrap();
                        std::net::Ipv4Addr::from(bytes).to_string()
                    }
                    3 => {
                        let length = client.read_u8().await.unwrap() as usize;
                        let mut bytes = vec![0u8; length];
                        client.read_exact(&mut bytes).await.unwrap();
                        String::from_utf8(bytes).unwrap()
                    }
                    4 => {
                        let mut bytes = [0u8; 16];
                        client.read_exact(&mut bytes).await.unwrap();
                        std::net::Ipv6Addr::from(bytes).to_string()
                    }
                    value => panic!("unsupported SOCKS address type {value}"),
                };
                let port = client.read_u16().await.unwrap();
                let mut upstream = TcpStream::connect((host.as_str(), port)).await.unwrap();
                client
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });
    (address, task)
}

fn candidate(address: std::net::SocketAddr, label: &str) -> UpstreamCandidate {
    UpstreamCandidate::from_url(
        Url::parse(&format!("http://{address}")).unwrap(),
        CandidateSource::Manual,
        label,
    )
    .unwrap()
}

fn healthy(candidate: UpstreamCandidate) -> CandidateHealth {
    CandidateHealth {
        candidate,
        state: HealthState::Healthy,
        latency_ms: Some(1),
        checked_at: chrono::Utc::now(),
        consecutive_failures: 0,
        failure: None,
        detail: None,
    }
}

async fn connect_relay(relay: std::net::SocketAddr, target: std::net::SocketAddr) -> TcpStream {
    let mut client = TcpStream::connect(relay).await.unwrap();
    client
        .write_all(
            format!(
                "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nConnection: keep-alive\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let header = read_header(&mut client).await;
    assert!(String::from_utf8_lossy(&header).contains(" 200 "));
    client
}

#[tokio::test]
async fn http_connect_tunnels_bytes() {
    let (echo, echo_task) = spawn_echo_server().await;
    let (proxy, count, proxy_task) = spawn_http_proxy(false).await;
    let mut tunnel = connect_tunnel(
        &candidate(proxy, "mock"),
        &echo.ip().to_string(),
        echo.port(),
    )
    .await
    .unwrap();
    tunnel.write_all(b"codex").await.unwrap();
    let mut response = [0u8; 5];
    tunnel.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"codex");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    proxy_task.abort();
    echo_task.abort();
}

#[tokio::test]
async fn socks5_tunnels_bytes_and_resolves_at_the_proxy() {
    let (echo, echo_task) = spawn_echo_server().await;
    let (proxy, proxy_task) = spawn_socks5_proxy().await;
    let socks = UpstreamCandidate::from_url(
        Url::parse(&format!("socks5h://{proxy}")).unwrap(),
        CandidateSource::Manual,
        "mock socks",
    )
    .unwrap();
    let mut tunnel = connect_tunnel(&socks, &echo.ip().to_string(), echo.port())
        .await
        .unwrap();
    tunnel.write_all(b"wss").await.unwrap();
    let mut response = [0u8; 3];
    tunnel.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"wss");
    proxy_task.abort();
    echo_task.abort();
}

#[tokio::test]
async fn proxy_407_is_reported() {
    let (proxy, _, proxy_task) = spawn_http_proxy(true).await;
    let error = connect_tunnel(&candidate(proxy, "rejecting"), "example.com", 443)
        .await
        .err()
        .expect("407 must be an error");
    assert!(error.to_string().contains("407"));
    proxy_task.abort();
}

#[tokio::test]
async fn disabled_direct_fallback_never_reaches_target() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_address = target.local_addr().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = listener.local_addr().unwrap();
    let runtime = ProxyRuntime::new(&GuardConfig::default());
    let relay_task = tokio::spawn(run_proxy(listener, runtime));

    let mut client = TcpStream::connect(relay).await.unwrap();
    client
        .write_all(
            format!("CONNECT {target_address} HTTP/1.1\r\nHost: {target_address}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let header = read_header(&mut client).await;
    assert!(String::from_utf8_lossy(&header).contains(" 502 "));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), target.accept())
            .await
            .is_err(),
        "the target received a direct connection"
    );
    relay_task.abort();
}

#[tokio::test]
async fn upstream_switch_only_affects_new_connections() {
    let (echo, echo_task) = spawn_echo_server().await;
    let (proxy_a, count_a, proxy_a_task) = spawn_http_proxy(false).await;
    let (proxy_b, count_b, proxy_b_task) = spawn_http_proxy(false).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = listener.local_addr().unwrap();
    let runtime = ProxyRuntime::new(&GuardConfig::default());
    runtime
        .replace_candidates(vec![healthy(candidate(proxy_a, "a"))])
        .await;
    let relay_task = tokio::spawn(run_proxy(listener, runtime.clone()));

    let mut existing = connect_relay(relay, echo).await;
    runtime
        .replace_candidates(vec![healthy(candidate(proxy_b, "b"))])
        .await;
    existing.write_all(b"old").await.unwrap();
    let mut old_response = [0u8; 3];
    existing.read_exact(&mut old_response).await.unwrap();
    assert_eq!(&old_response, b"old");

    let mut new_connection = connect_relay(relay, echo).await;
    new_connection.write_all(b"new").await.unwrap();
    let mut new_response = [0u8; 3];
    new_connection.read_exact(&mut new_response).await.unwrap();
    assert_eq!(&new_response, b"new");
    assert_eq!(count_a.load(Ordering::SeqCst), 1);
    assert_eq!(count_b.load(Ordering::SeqCst), 1);

    relay_task.abort();
    proxy_a_task.abort();
    proxy_b_task.abort();
    echo_task.abort();
}

#[tokio::test]
async fn failed_vpn_port_falls_through_to_the_new_healthy_port() {
    let (echo, echo_task) = spawn_echo_server().await;
    let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = closed_listener.local_addr().unwrap();
    drop(closed_listener);
    let (recovered_proxy, recovered_count, recovered_task) = spawn_http_proxy(false).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = listener.local_addr().unwrap();
    let runtime = ProxyRuntime::new(&GuardConfig::default());
    runtime
        .replace_candidates(vec![
            healthy(candidate(closed_port, "old VPN port")),
            healthy(candidate(recovered_proxy, "new VPN port")),
        ])
        .await;
    let relay_task = tokio::spawn(run_proxy(listener, runtime));

    let mut connection = connect_relay(relay, echo).await;
    connection.write_all(b"recovered").await.unwrap();
    let mut response = [0u8; 9];
    connection.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"recovered");
    assert_eq!(recovered_count.load(Ordering::SeqCst), 1);

    relay_task.abort();
    recovered_task.abort();
    echo_task.abort();
}
