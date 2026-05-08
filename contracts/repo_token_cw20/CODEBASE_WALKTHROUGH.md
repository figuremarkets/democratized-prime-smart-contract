# Repo Token CW20 Codebase Walkthrough

This document walks through the **repo_token_cw20** contract: a custom CW20 used as the pool’s receipt token. Balances are stored in **scaled** form; when `pool_address` is set, the contract queries the pool for `liquidity_index` and returns **underlying** in Balance and TokenInfo so wallets show real value.

**Deployment:** See **[POOL_AND_REPO_TOKEN_DEPLOYMENT.md](../../docs/POOL_AND_REPO_TOKEN_DEPLOYMENT.md)** for two ways to wire pool_v2: **`repo_token.new`** (pool instantiates this contract in a submessage) vs **`repo_token.existing`** plus **UpdateConfig**.

---

## 1. State (`src/state.rs`)

What gets stored on-chain.

| Item | Type | Purpose |
|------|------|--------|
| **BALANCES** | `Map<Addr, Uint128>` | Scaled balance per address. Balance query returns this (or underlying when pool is set). |
| **TOKEN_INFO** | `TokenInfo` | Name, symbol, decimals, and **total_supply** (scaled; TokenInfo query may return underlying when pool is set). |
| **CONFIG** | `Config` | Admin (may call UpdateConfig), minter (only address that may Mint/Burn), and optional pool_address (for underlying conversion). |

- **TokenInfo** holds `total_supply` in scaled units; it is updated on every Mint/Burn.
- **Config** is set at instantiation and updated only via **UpdateConfig** (admin only).

---

## 2. Messages (`src/msg.rs`)

**InstantiateMsg:** Defined in **`democratized_prime_lib::repo_token`** and **re-exported** here (name, symbol, decimals, admin, minter, optional pool_address). No initial supply. Keeping a single Rust type ensures **`pool_v2`**’s instantiate SubMsg and this contract’s `instantiate` entry point agree on JSON and field names.

**ExecuteMsg:**

| Variant | Who | Purpose |
|--------|-----|--------|
| Mint | Minter only | Increase balance and total_supply (scaled). Pool calls this when a user lends. |
| Burn | Minter only | Decrease **sender’s** balance and total_supply (scaled). Pool calls this on **user** Withdraw: user Sends repo token to the pool, pool burns that received balance. |
| BurnFrom | Minter only | Decrease **another address’s** balance and total_supply (scaled). Pool calls this on **admin** Withdraw: admin withdraws a lender’s supply on their behalf; no token is sent to the pool, pool burns from the lender’s balance. |
| Transfer | Any (restricted) | Move scaled amount from sender to recipient. **Users** may only transfer **to the pool**; the pool may transfer to any address. |
| Send | Any (restricted) | Same gating: users may only send **to the pool**; pool may send to any contract with payload. |
| UpdateConfig | Admin only | Set minter and/or pool_address (required when the token was created before the pool; optional if the pool already passed minter + pool_address at instantiate). |

**QueryMsg:** Balance (address), TokenInfo, Minter. Same JSON shape as standard CW20 so wallets work.

---

## 3. Query (`src/query.rs`)

- **Balance:** Load scaled balance. If `config.pool_address` is set, call **pool_query::query_liquidity_index**, compute underlying = scaled × index (floor), return that; otherwise return scaled.
- **TokenInfo:** Load token info and total_supply. If pool_address is set, convert total_supply to underlying the same way; otherwise return scaled total_supply.
- **Minter:** Return config.minter (no cap).

All use standard CW20 response types (`BalanceResponse`, `TokenInfoResponse`, `MinterResponse`).

---

## 4. Pool query (`src/pool_query.rs`)

- **PoolReserveResponse** / **PoolReserveState:** Minimal structs to deserialize the pool’s **GetReserve** response (we only need `reserve.liquidity_index`).
- **query_liquidity_index(querier, pool_address):** Sends `GetReserve` to the pool, parses the JSON, returns `Decimal256` liquidity index. Used by Balance and TokenInfo when pool_address is set.

The pool’s query uses `get_reserve` (snake_case); our **PoolQueryMsg** enum serializes to that.

---

## 5. Execute (`src/execute.rs`)

- **Mint:** Check sender == minter. Add amount to recipient’s balance and to total_supply. Amount is scaled.
- **Burn:** Check sender == minter. Subtract amount from sender’s balance and from total_supply. Used when the pool has received repo token via Send (user withdraw) and burns its own balance.
- **BurnFrom:** Check sender == minter. Subtract amount from **owner's** balance and from total_supply. Used when the pool (admin withdraw) withdraws a lender's supply on their behalf—no token is sent; the pool burns from the lender's address.
- **Transfer:** Enforce **transfer/send gating** (see below); then subtract from sender, add to recipient (scaled).
- **Send:** Same gating; then transfer to contract and emit **Cw20ReceiveMsg** to that contract.
- **ensure_transfer_send_allowed:** If sender is not the pool or minter, recipient must be the pool (else `IllegalArgument`). If `pool_address` is not set, any such transfer/send fails with `PoolNotConfigured`. This ensures user-to-user flows go through the pool so the pool can enforce lender attributes on recipients.
- **UpdateConfig:** Check sender == admin. Optionally update minter and/or pool_address (validate addresses). Save config. Used for the “deploy CW20 first” path; the “new with pool” path sets these fields in `InstantiateMsg`.

All balance changes use scaled amounts. Overflow from checked_add/checked_sub is mapped to **ContractError::OverflowErr**.

---

## 6. Instantiate (`src/instantiate.rs`)

Call **`democratized_prime_lib::repo_token::validate_repo_token_meta`** on name/symbol/decimals (same rules as cw20-base; shared with **`pool_v2`** for `repo_token.new`). Validate admin, minter, and optional pool_address. Save **Config** and **TokenInfo** (total_supply = 0). Set contract version (cw2). No balances created.

---

## 7. Utils (`src/utils.rs`)

**scaled_to_underlying_floor(scaled, liquidity_index):** Computes underlying = scaled × liquidity_index with **floor** rounding. Floor ensures we never over-state the user's balance (dust stays in the pool; consistent with pool_v2's `scaled_to_underlying_liquidity` used for balance queries and withdraw limits). Uses Decimal256 and 18-decimal truncation; returns u128. Used by Balance and TokenInfo when returning underlying.

---

## 8. Entry points (`src/contract.rs`)

Thin wrappers: **instantiate**, **execute**, **query** call into the modules above. Entry points are gated with `#[cfg(not(feature = "library"))]` so the crate can be used as a library (e.g. for tests).

---

## 9. Tests (`src/tests.rs`)

Unit tests focus on **custom behavior**: Balance/TokenInfo with and without pool (mock pool GetReserve), and auth (mint/burn only minter, UpdateConfig only admin). Standard CW20 Transfer/Send balance math is not re-tested. Uses valid Provenance bech32 (tp1) addresses so addr_validate passes.
