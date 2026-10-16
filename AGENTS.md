## Definition of done

### Per task

- `cargo fmt --all -- --check` — formatting (source: Makefile target `fmt`)
- `cargo clippy` — lints (source: Makefile target `lint`)
- `cargo test` — tests (source: Makefile target `test`; CI: `build`, `.github/workflows/build.yaml` step Test)

### Before opening a PR

- `make all` — clean, fmt, lint, test, schema, and optimized WASM (source: Makefile target `all`)

## Agent environment

What a fresh machine (a cloud agent, a new laptop) needs before the commands above start. Names only, never values.

- Toolchain: Rust stable and Cargo (source: `README.md` Prerequisites). CI installs Rust with `actions-rust-lang/setup-rust-toolchain@v1` and no `rust-toolchain` file (source: `.github/workflows/build.yaml`).
- Credentials: none for the commands above. `PIO_HOME` is only for `make install` (source: Makefile target `install`).
- Docker: Docker or Podman for `make optimize`, which `make all` runs (source: Makefile `CONTAINER_RUNTIME` and `README.md` Prerequisites). `cargo test` does not use a container.
- Services: none. No compose file in the repository; tests use in-process mocks.
- Tools: `make` and Cargo, including rustfmt and clippy (source: Makefile targets `fmt`, `lint`, `test`, `schema`, `all`).
- Cloud agents: not configured (no `.cursor/environment.json`)

## Protected paths

### Escalate, don't change

- `contracts/*/src/storage/**` — storage layout and key strings (e.g. `"cs1"`, `"res1"`, `"sb1"`); renaming a key or changing a stored type orphans on-chain data (source: `Item::new` / `Map::new` in `contracts/pool_v2/src/storage/` and `contracts/price_oracle/src/storage/`)
- `contracts/repo_token_cw20/src/state.rs` — CW20 storage layout and keys (`"balances"`, `"token_info"`, `"config"`) (source: `Map::new` / `Item::new` in that file)

### Regenerate, never hand-edit

- `contracts/*/schema/**` — `make schema` (source: Makefile target `schema`)

## Agent tooling

- MCP servers: none (no `.mcp.json` or `.cursor/mcp.json`)
- Plugins: `ai-ready-playbook`, `agent-workflow`, `shortcut` — story-to-PR workflow (commit subjects use `[sc-…]`)

## Common AI mistakes

Add a row when agents make the same mistake here more than once, and point at the section that states the right behaviour.

| Date | Mistake | Correct behaviour | Where it is written |
|------|---------|-------------------|---------------------|
| 2026-10-02 | Comments cited Shortcut story IDs | Describe the behavior in code terms | `## Public repository` |
| 2026-10-02 | Comments described deployment context instead of behavior | State what the code or test asserts | `## Public repository` |

## Public repository

Everything committed here is public. Comments and documentation describe the code only.

- Describe what the code does; keep product plans and future work in Shortcut.
- Leave out operational details and production posture.
- Report known defects and exploit conditions privately, following `.github/SECURITY.md`.
- Shortcut ids and links appear only in branch names, commit subjects, pull request titles, and the trailing `Shortcut:` line.
- Keep discussion notes, audit reproductions, and unpublished findings out of the repository. Published audits stay (`contracts/repo_token_cw20/CW20_AUDIT.md`).
