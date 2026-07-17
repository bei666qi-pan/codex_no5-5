# Testing and acceptance

## Automated coverage

The current suite covers:

- PAC route parsing and relay-loop rejection
- candidate ordering and credential redaction, including serialized status/diagnostic output
- failure classification, user-facing next-step guidance, and JSON-RPC version rejection
- HTTP CONNECT byte tunnelling and upstream HTTP 407
- disabled direct fallback with a target-side leakage assertion
- upstream replacement affecting only new connections
- LaunchAgent XML escaping and reversible terminal block removal
- proxy environment injection helpers and configuration-safe defaults

Run all checks with:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Manual macOS matrix

Before a public beta, exercise both Apple Silicon and Intel on macOS 13 or newer with:

| Client | Explicit HTTP | PAC | Mixed port | SOCKS5 only | Port change | Restart |
| --- | --- | --- | --- | --- | --- | --- |
| Clash Verge Rev | required | required | required | required | required | required |
| ClashX | required | required | when supported | required | required | required |
| Surge | required | required | when supported | required | required | required |
| V2RayU | required | when supported | when supported | required | required | required |

Also verify current and previous Codex versions, App and CLI launch, missing `doctor`, missing `respect_system_proxy`, and missing `remote-control` degradation.

On Windows 10/11 x64, verify the following in both a standard user account and a non-English display language:

- Explicit Windows Internet Settings HTTP proxy, SOCKS entry and AutoConfigURL PAC.
- VPN port change and restart recovery without restarting the guard.
- Task Scheduler creation, login restart, uninstall restoration of `CODEX_CLI_PATH`, and absence of a global WinHTTP proxy change.
- Desktop, CLI, named-pipe `status --json`, remote-control feature fallback, and a portable ZIP launch with WebView2.
- Desktop guidance actions for VPN unavailable, paused protection, 401/403, 429, 5xx, and Codex process failures; verify that the exported diagnostic contains no proxy credentials.

## Release gates

- First-time setup within two minutes and at most three clicks.
- A changed proxy route is selected for the next connection within five seconds.
- VPN restart requires no guard restart.
- Twenty-four-hour HTTPS/WSS load test with no meaningful memory growth and idle CPU below 0.5%.
- Login reboot restores the daemon and enabled remote-control process.
- Install/uninstall restores external environment and shell state.
- Developer ID signature, notarization and update signature are mandatory before calling a build beginner-ready.
