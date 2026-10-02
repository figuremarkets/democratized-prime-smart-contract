## Definition of done

### Per task

- `cargo fmt --all -- --check` — formatting (source: Makefile target `fmt`)
- `cargo clippy` — lints (source: Makefile target `lint`)
- `cargo test` — tests (source: Makefile target `test`; CI `.github/workflows/build.yaml`)

### Before opening a PR

- `make all` — clean, fmt, lint, test, schema, optimized WASM; needs Docker or Podman (source: Makefile target `all`)

## Protected paths

### Escalate, don't change

- `contracts/*/src/storage/**` — storage layout and key strings (e.g. `"cs1"`, `"res1"`, `"sb1"`); renaming a key or changing a stored type orphans on-chain data
- `contracts/repo_token_cw20/src/state.rs` — CW20 storage layout and keys (`"balances"`, `"token_info"`, `"config"`)

### Regenerate, never hand-edit

- `contracts/*/schema/**` — `make schema`

## Public repository

Everything committed here is public: code, comments, docs, commit messages, branch names, PR titles and descriptions.

- Comments and docs describe what the code does and why, in code terms. No product plans, future features, partner, treasury or operational details, and no "production" posture.
- Never describe a known defect, an exploit, or the inputs or conditions that reach one. Fix it, or report it privately per `.github/SECURITY.md`.
- No Shortcut links or story IDs in code, comments, docs, or PR description text. They're allowed only in commit subjects, branch names, PR titles, and the trailing `Shortcut:` line of the PR description.
- Never commit discussion notes or unpublished findings. Published audits (`contracts/repo_token_cw20/CW20_AUDIT.md`) stay.

## Agent tooling

- MCP servers: none
- Plugins: `ai-ready-playbook`, `agent-workflow`, `shortcut` — Figure's story-to-PR workflow (commits use `[sc-…]` subjects)
- Cloud environment: not configured (needs Rust stable + Cargo; Docker or Podman for `make optimize`)

## Common AI mistakes

| Date | Mistake | Correct behavior | Where it is written |
|------|---------|------------------|---------------------|
| 2026-10-02 | Comments cited Shortcut story IDs | Describe the behavior in code terms | `## Public repository` |
| 2026-10-02 | Comments described deployment context instead of behavior | State what the code or test asserts | `## Public repository` |
