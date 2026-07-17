# Architecture

## Components

| Component | Responsibility |
| --- | --- |
| `cng-core` | Configuration, discovery, proxy protocols, health scoring, diagnostics, JSON-RPC, Codex wrapping and service management |
| `cngd` | Loopback relay, five-second refresh loop, remote-control child supervision and private logs |
| `cng` | Stable CLI and JSON output |
| `cng-codex` | Transparent `CODEX_CLI_PATH` and terminal wrapper |
| `cng-desktop` | Tauri 2 menu-bar application and first-run flow |

## Data and control planes

The relay is deliberately a raw tunnel rather than a TLS-terminating proxy. For `CONNECT`, it establishes the selected HTTP/HTTPS/SOCKS5 upstream tunnel, returns `200 Connection Established`, then copies bytes bidirectionally. For absolute-form plain HTTP, it strips proxy-only headers and forwards origin-form requests through the same tunnel.

The control plane is JSON-lines RPC v1 over a per-user Unix Socket on macOS and a local Windows named pipe on Windows. The Unix Socket containing directory is `0700` and the socket is `0600`; the Windows pipe is not a network listener. Public enums use snake-case serialized names. v1 additions must use optional/defaultable fields; existing fields and meanings must not be removed or changed.

## Candidate selection

Each refresh discovers candidates, probes them concurrently with a bounded timeout and sorts by:

1. `healthy`, `degraded`, `unknown`, `down`
2. manual, system PAC, system proxy, environment, known loopback
3. measured latency

A previously working candidate gets one degraded grace result before becoming down. Down candidates are not used for new connections. The relay state is atomically replaced for future connections, while connection tasks retain their existing tunnel.

## Privacy model

The guard records operational metadata only. It never terminates destination TLS, so it cannot read URL paths inside HTTPS, request bodies, Codex conversations or bearer tokens. Error formatting removes proxy URL credentials and replaces the home directory with `~`. A credential-bearing manual proxy URL is stored in Keychain rather than TOML.

## macOS integration

On macOS, the LaunchAgent owns only `dev.codex-network-guard.daemon`. On Windows, a per-user Task Scheduler task named `CodexNetworkGuard` owns only `cngd.exe`; administrator rights are not required. Installation stores the previous `CODEX_CLI_PATH`, points it at `cng-codex`, and restores the prior value only if it still equals the guard wrapper during uninstall. This prevents overwriting a later user change.

Terminal integration is opt-in and consists of one removable block in `~/.zprofile`. Legacy migration is also opt-in: the new service must already be installed and running before the old plist is backed up and booted out.
