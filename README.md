# Democratized Prime — Smart Contracts

CosmWasm contracts for **Democratized Prime** lending: an index-based pool with a kink interest-rate model, a receipt **CW20** that tracks scaled lender balances against the pool’s liquidity index, a **price oracle**, and shared Rust types in a workspace library. The stack targets chains where **Provenance** modules (markers, exchange, etc.) are available via `provwasm-std`.

This repository is intended to be **publicly readable** under the **Business Source License 1.1** (see [License](#license) below).

### Why this is public (governance and audit)

The code is here so **governance** participants and the **community** can **review and audit** what they are voting on: the behavior of the contracts that **main-chain proposals** would deploy or upgrade, and how the product is meant to work. **We do not solicit external involvement**—no pull requests, feature design collaboration, or informal contributions. Development stays **internal** to Figure Markets. For **production use** or **commercial licensing**, see [LICENSE](LICENSE).

---

## What’s in this repo

| Component | Path | Role |
|-----------|------|------|
| **Pool v2** | [`contracts/pool_v2/`](contracts/pool_v2/) | Lend, borrow, repay, collateral, liquidation, reserve accrual, optional Provenance **commit-on-exit** flows. Behavior: [CODEBASE_WALKTHROUGH.md](contracts/pool_v2/CODEBASE_WALKTHROUGH.md). |
| **Repo token (CW20)** | [`contracts/repo_token_cw20/`](contracts/repo_token_cw20/) | Receipt token: scaled balances; when wired to the pool, queries expose **underlying** amounts using the pool’s liquidity index. |
| **Price oracle** | [`contracts/price_oracle/`](contracts/price_oracle/) | Asset mappings and USD price data for pool risk and valuation. |
| **Shared library** | [`packages/lib/`](packages/lib/) | Common types (e.g. repo token `InstantiateMsg` and validation) used by more than one contract. |

JSON **schemas** for each contract live under each contract’s `schema/` directory (regenerated via the `schema` example targets).

---

## Documentation

| Doc | Description |
|-----|-------------|
| [docs/POOL_AND_REPO_TOKEN_DEPLOYMENT.md](docs/POOL_AND_REPO_TOKEN_DEPLOYMENT.md) | How **pool_v2** binds to **repo_token_cw20** (new token vs existing token), naming conventions, and CLI-oriented steps. |
| [docs/CLI_TEST_POOL_V2.md](docs/CLI_TEST_POOL_V2.md) | CLI-focused testing notes for pool v2. |
| [contracts/pool_v2/CODEBASE_WALKTHROUGH.md](contracts/pool_v2/CODEBASE_WALKTHROUGH.md) | High-level walkthrough of pool v2 models, messages, and execution flow. |
| [contracts/repo_token_cw20/CODEBASE_WALKTHROUGH.md](contracts/repo_token_cw20/CODEBASE_WALKTHROUGH.md) | Walkthrough of the repo CW20 contract. |

---

## Prerequisites

- **Rust** (stable), **Cargo**, and standard CosmWasm development tooling.
- **Docker** or **Podman** (for optimized WASM builds via the official CosmWasm optimizer image).

---

## Quick start

Clone the repository and run tests from the workspace root:

```bash
cargo test
```

Format and lint (same checks useful before pushing):

```bash
cargo fmt --all
cargo clippy
```

Regenerate JSON schemas:

```bash
make schema
```

Or run each schema example explicitly:

```bash
cargo run -p democratized-prime-pool-v2 --example schema
cargo run -p democratized-prime-price-oracle --example schema
cargo run -p repo-token-cw20 --example schema
```

### Optimized WASM artifacts

The [Makefile](Makefile) targets `optimize` and `optimize-arm` run the **cosmwasm/optimizer** (or **rust-optimizer-arm64**) image against the workspace. With the default workspace layout, you should get WASM binaries under **`artifacts/`**, including:

- `democratized_prime_pool_v2.wasm`
- `repo_token_cw20.wasm`
- `democratized_prime_price_oracle.wasm`

The `install` / `install-arm` targets copy those files to **`$(PIO_HOME)`** (set that environment variable if you use them).

```bash
make optimize
```

---

## License

Licensed under the **Business Source License 1.1** (`BUSL-1.1`). The full terms and **Parameters** (Licensor, Licensed Work, Additional Use Grant, Change Date, Change License) are in the [LICENSE](LICENSE) file in this repository.

**In short (not a substitute for the license text):**

- You may use, study, modify, and redistribute the code for **non-production** purposes under BSL, subject to the full license.
- This repository’s **Additional Use Grant** is **None**. **Production** use generally requires a **commercial license** from the Licensor (see the contact line in [LICENSE](LICENSE)) unless and until the **Change License** applies.
- **After** the **Change Date** (or the fourth anniversary of public distribution of a given version, whichever comes first, as described in the license), that version is intended to be available under the **Change License** stated in [LICENSE](LICENSE) (currently **Apache License, Version 2.0** for this repo’s Parameters).

BSL is **not** the same as an OSI “open source” license **until** the Change License takes effect for a given version. For the canonical license text and MariaDB’s trademark/covenant terms, see [LICENSE](LICENSE).

---

## Security

To report a **vulnerability** in these contracts, follow [`.github/SECURITY.md`](.github/SECURITY.md). Do not use public issues for undisclosed security bugs.

---

## Disclaimer

This software is provided **as is**, without warranty of any kind. Smart contracts hold and move value; deploying or interacting with them involves **risk**. This README does not constitute legal, financial, or security advice. Obtain a professional **audit** and legal review before mainnet use. The authors and Licensor are **not** responsible for losses arising from use of this code.
