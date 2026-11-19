# Review rules

- Storage layout (`contracts/*/src/storage/**`, `contracts/repo_token_cw20/src/state.rs`): flag any change to a storage key string or stored type that has no matching migration in `src/msg/migrate.rs`. (AGENTS.md › Protected paths)
- Schemas (`contracts/*/schema/**`): flag schema changes that were not produced by `make schema`. (AGENTS.md › Protected paths)
- Public repository: flag comments, docs, or pull request text that include Shortcut ids or links, product plans or future work, operational details or production posture, a known defect or exploit condition, or discussion notes, audit reproductions, and unpublished findings. (AGENTS.md › Public repository)
- **Pull request base branch:** Flag a pull request whose base is not the branch its head was started from. New work starts from the default branch. A child branch targets its parent. A base named in the pull request description is acceptable (source: `AGENTS.md` — `## Pull request base`).
