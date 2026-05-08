# Repo Token CW20

Custom CW20 used as the pool’s receipt token. Balances are stored in **scaled** form; the contract can report **underlying** (scaled × pool liquidity index) so wallets show real value.

**Doc naming:** This README is the main entry point (overview, behavior, deployment). For a guided tour of the code, see **[CODEBASE_WALKTHROUGH.md](CODEBASE_WALKTHROUGH.md)** (same convention as pool_v2). For a comparison with the canonical cw20-base and what we omit by design, see **[CW20_AUDIT.md](CW20_AUDIT.md)**.

**CW20 compatibility:** Wallets recognize this as a CW20 because it implements the standard interface: query messages `Balance`, `TokenInfo`, and `Minter` with the same JSON structure and the same response types (`cw20::BalanceResponse`, `TokenInfoResponse`, `MinterResponse`). Execute messages `Transfer` and `Send` also follow the CW20 spec. There is no on-chain “CW20 type” flag—wallets that support CW20 call these queries; if the contract responds in the expected format, they treat it as a CW20 token.

**CW20 standard and reference:** This contract is a custom implementation that follows the CW20 interface; it is not a fork of the reference contract. The canonical reference and shared types come from the CosmWasm ecosystem:
- **Reference implementation (cw20-base):** [CosmWasm/cw-plus – contracts/cw20-base](https://github.com/CosmWasm/cw-plus/tree/main/contracts/cw20-base)
- **cw20 crate (message/response types):** [CosmWasm/cw-plus – packages/cw20](https://github.com/CosmWasm/cw-plus/tree/main/packages/cw20) (we depend on this for `BalanceResponse`, `TokenInfoResponse`, `Cw20QueryMsg`, etc.)

**Why not extend cw20-base?** We use only the `cw20` types and implement the logic ourselves instead of importing `cw20_base::contract::handle_transfer` etc. Our contract has different semantics (minter-only burn, admin UpdateConfig, no allowances) and a different instantiate/state (admin, pool_address, no initial_balances). Extending base would mean overriding instantiate, burn, and queries and stripping or ignoring allowances/marketing—so we’d reuse only transfer/send/mint while carrying the rest. For this minimal receipt token, a custom implementation keeps the surface small and the behavior explicit. See **[CW20_AUDIT.md](CW20_AUDIT.md)** for a full comparison.

## Behavior

- **Storage:** Balances and total supply are stored in scaled units (same as pool accounting).
- **Balance / TokenInfo:** When `pool_address` is set, `Balance` and `TokenInfo.total_supply` query the pool’s `GetReserve`, use `liquidity_index`, and return **underlying** (floor rounding). Otherwise they return scaled.
- **Mint / Burn / BurnFrom:** Only the **minter** (the pool contract) may mint, burn (its own balance), or **BurnFrom** (another address's balance—used when the pool admin withdraws a lender's supply on their behalf).
- **Transfer / Send:** Amounts are in scaled units. Users may only transfer or send **to the pool**; the pool (minter) may transfer/send to any address. The pool forwards to recipients after checking lender attributes, so all user-to-user flows go through the pool.
- **UpdateConfig:** **Admin** may set `minter` and/or `pool_address` (needed on the “deploy repo before pool” path; not needed for initial wiring when **pool_v2** created this contract via **`repo_token.new`**).

## Deployment with pool_v2 (`repo_token_cw20`)

See **[POOL_AND_REPO_TOKEN_DEPLOYMENT.md](../../docs/POOL_AND_REPO_TOKEN_DEPLOYMENT.md)** for diagrams, JSON, and rationale.

**Path 1 — Pool creates the repo token (`pool_v2` `InstantiateMsg.repo_token.new`):** You only instantiate **pool_v2**. The pool sends `WasmMsg::Instantiate` for this contract with `admin` = pool instantiator, `minter` = pool, `pool_address` = pool. **No UpdateConfig** is required for mint/burn or underlying Balance/TokenInfo.

**Path 2 — You deploy this CW20 first (`repo_token.existing` on the pool):**

1. **Instantiate this CW20** with `admin`, `minter` = admin, `pool_address` = `None`.
2. **Instantiate pool_v2** with `repo_token.existing.repo_token_cw20_contract_address` = this CW20’s address.
3. **Execute UpdateConfig on this CW20** (as admin): set `minter` and `pool_address` to the pool’s address. Only after this can the pool mint/burn and Balance/TokenInfo return underlying.

## Dependencies

- `cosmwasm-std`, `cw20`, `cw-storage-plus`, `cw2`, `provwasm-std`, `schemars`, `serde`, `thiserror`.
- **`democratized-prime-lib`** — defines **`InstantiateMsg`** and **`validate_repo_token_meta`** in **`repo_token`**. This crate re-exports **`InstantiateMsg`** from `msg.rs` so JSON matches **`pool_v2`**’s `WasmMsg::Instantiate` submessage when using **`repo_token.new`**.
