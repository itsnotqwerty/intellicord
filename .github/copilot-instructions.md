# Callmon Development

- [x] Requirements clarified: Rust voice-call history with a Discord-style injected menu.
- [x] Project scaffolded and customized with a Rust core, opt-in DevTools adapter, and shared JavaScript menu.
- [x] Dependencies installed; no additional VS Code extensions required.
- [x] Rust and JavaScript behavioral tests run; interactive preview checked in a browser.
- [x] Preview launched without touching the user's authenticated Discord process.
- [x] Build, attachment, privacy, and compatibility instructions documented in README.md.

- Keep client modifications opt-in. Do not launch, patch, or terminate the user's Discord process without approval.
- Keep DevTools connections loopback-only. Never read authentication tokens or record audio/messages.
- Keep Discord's private-store assumptions inside src/web/bridge.js and fail visibly on unsupported builds.
- Build with npm ci, npm run build, then cargo build. The client bundle is embedded in Rust.
- Verify with npm test, cargo test, cargo fmt --check, and cargo clippy --all-targets -- -D warnings.
- Preserve string user IDs, baseline semantics, memory-only bounded history, and the existing compact menu style.
- Live attachment and active-call detection were verified on Discord stable desktop 1.0.169, web build 673869. Other builds and call types remain unverified; mocked tests alone are not proof of live compatibility.