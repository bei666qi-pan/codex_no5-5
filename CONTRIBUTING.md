# Contributing

Keep changes narrowly scoped and preserve the project's privacy boundary: no TLS interception, token capture, global system proxy changes or silent direct fallback.

Before opening a pull request, run formatting, Clippy and the full workspace test suite. Network behavior changes should include a local mock integration test and must not depend on live OpenAI services in CI.

Public JSON output and JSON-RPC v1 are compatibility surfaces. Add fields in a backward-compatible way and do not rename serialized enum variants in a patch release.
