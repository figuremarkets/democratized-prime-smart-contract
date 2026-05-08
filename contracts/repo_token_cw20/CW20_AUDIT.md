# repo_token_cw20 vs cw20-base Audit

This document compares **repo_token_cw20** to the canonical [cw20-base](https://github.com/CosmWasm/cw-plus/tree/main/contracts/cw20-base) implementation and the [cw20](https://github.com/CosmWasm/cw-plus/tree/main/packages/cw20) message spec. It clarifies what we implement, what we omit by design, and any optional improvements.

---

## 1. Execute messages

| Message | cw20-base | repo_token_cw20 | Notes |
|--------|-----------|------------------|--------|
| **Transfer** | ✅ Any holder → any recipient | ✅ **Restricted** | Users may only transfer **to the pool**; pool (minter) may transfer to any address. Amount in scaled units. See “Transfer/Send gating” below. |
| **Send** | ✅ Any holder → any contract | ✅ **Restricted** | Same gating: users may only send **to the pool**; pool may send to any contract. We build `Cw20ReceiveMsg` and `WasmMsg::Execute` correctly. |
| **Burn** | ✅ Any holder (own balance) | ✅ **Minter only** | Intentional: only the pool burns (on Withdraw). Users withdraw via pool, not self-burn. |
| **Mint** | ✅ Minter only | ✅ Minter only | Same. |
| **IncreaseAllowance** | ✅ (allowance extension) | ❌ Not implemented | Omitted by design: receipt tokens typically don’t need approvals; pool is sole minter/burner. |
| **DecreaseAllowance** | ✅ | ❌ | Same as above. |
| **TransferFrom** | ✅ | ❌ | Same as above. |
| **SendFrom** | ✅ | ❌ | Same as above. |
| **BurnFrom** | ✅ (with allowance) | ✅ **Minter only** | We implement BurnFrom { owner, amount }: only minter may call it; no allowance. Used when the pool (admin withdraw) withdraws a lender's supply on their behalf. |
| **UpdateMinter** | ✅ Minter can set new minter | ❌ | We use **UpdateConfig** (admin only) to set minter + pool_address. Fits the **existing-token** deploy path; when **pool_v2** uses **`repo_token.new`**, the pool passes minter + pool_address at **InstantiateMsg** so **UpdateConfig** is not required for the initial wire-up. |
| **UpdateMarketing** | ✅ (marketing extension) | ❌ | Omitted: no marketing metadata for receipt tokens. |
| **UploadLogo** | ✅ | ❌ | Same as above. |

**Transfer/Send gating:** When `pool_address` is set, only the **pool** (or minter) may transfer or send to arbitrary addresses. Any other sender may transfer/send **only to the pool**. The pool then forwards (e.g. via its Transfer execute) after checking lender attributes. This keeps receipt-token flows going through the pool so the pool can enforce policy (e.g. lender-attr on recipients). If `pool_address` is not set, any transfer/send to a non-pool address fails with `PoolNotConfigured`.

**Receive (Cw20ReceiveMsg):** The **token** contract never receives `Cw20ReceiveMsg`. When a user does **Send**, the token contract executes **Send** and then sends a **submessage** to the **receiver** contract with `Cw20ReceiveMsg`. So repo_token_cw20 does not need a Receive handler; pool_v2 (the receiver) implements it for Withdraw/Transfer payloads.

---

## 2. Query messages

| Query | cw20-base | repo_token_cw20 | Notes |
|-------|-----------|------------------|--------|
| **Balance** | ✅ Raw balance | ✅ **Scaled or underlying** | When `pool_address` is set we return underlying (scaled × liquidity_index, floor). Else scaled. Same response type `BalanceResponse`. |
| **TokenInfo** | ✅ name, symbol, decimals, total_supply | ✅ Same | total_supply is underlying when pool is set, else scaled. Same response type `TokenInfoResponse`. |
| **Minter** | ✅ minter, cap | ✅ minter, **cap: None** | We don’t enforce a cap; MinterResponse is compatible. |
| **Allowance** | ✅ | ❌ | Not implemented (no allowance extension). |
| **AllAllowances / AllSpenderAllowances** | ✅ | ❌ | Same. |
| **AllAccounts** | ✅ (enumerable) | ❌ | Not implemented; not required for receipt token. |
| **MarketingInfo** | ✅ | ❌ | No marketing extension. |
| **DownloadLogo** | ✅ | ❌ | Same. |

---

## 3. Instantiate

| Field / behavior | cw20-base | repo_token_cw20 | Notes |
|------------------|-----------|------------------|--------|
| name | Required, validated 3–50 UTF-8 bytes | Required, validated (shared) | Same rules via **`democratized_prime_lib::repo_token::validate_repo_token_meta`**. |
| symbol | Required, validated 3–12, `[a-zA-Z\-]` | Required, validated (shared) | Same. |
| decimals | Required, ≤ 18 | Required, ≤ 18 (shared) | Same. |
| initial_balances | Optional list of (address, amount) | **None** | By design: no initial balances; total supply starts at 0; the pool mints when lenders lend. |
| mint | Optional { minter, cap } | Replaced by **admin + minter** in config | We use CONFIG (admin, minter, pool_address). |
| marketing | Optional | **None** | No marketing. |
| pool_address | N/A | **Optional** | Our extension; set at instantiate when the pool creates the CW20 (`repo_token.new`), or later via **UpdateConfig** on the existing-token path. |

---

## 4. State and response types

- We use the same **response types** from the `cw20` crate: `BalanceResponse`, `TokenInfoResponse`, `MinterResponse`. Wallets and indexers that expect these get compatible JSON.
- **BALANCES** and **TOKEN_INFO** match the logical fields; we add **CONFIG** (admin, minter, pool_address) instead of embedding minter in token info.

---

## 5. Summary: what we’re “missing” and why

- **Allowances (Increase/DecreaseAllowance, TransferFrom, SendFrom, allowance-based BurnFrom, Allowance queries):** Not implemented. We do implement **BurnFrom** as minter-only (burn from any address without allowance) for admin withdraw. Standard for many CW20s; omitted here because receipt tokens are pool-managed and don’t need delegated spending. If a future use case needs approvals, they can be added as an extension.
- **UpdateMinter:** Replaced by admin-only **UpdateConfig** (minter + pool_address) for the manual deploy path; the pool-driven instantiate path sets both at **InstantiateMsg**.
- **Marketing / Logo:** Not implemented; not needed for receipt tokens.
- **Enumerable (AllAccounts):** Not implemented; not required for current use.
- **Instantiate validation:** Name/symbol/decimals are validated against cw20-base-style rules in **`democratized_prime_lib::repo_token::validate_repo_token_meta`**, called from **`instantiate::instantiate`**. **`pool_v2`** calls the same helper when **`repo_token.new`** is used so off-chain clients and the SubMsg stay consistent.

---

## 6. Instantiate validation (implemented)

Validation lives in **`democratized_prime_lib::repo_token::validate_repo_token_meta`** and is invoked from **`instantiate::instantiate`** to align with cw20-base:

- **Name:** 3–50 UTF-8 bytes.
- **Symbol:** 3–12 bytes, only `[a-zA-Z\-]`.
- **Decimals:** ≤ 18.

On failure this contract maps errors to **`ContractError::IllegalArgument`**. Unit tests cover: name too short/long, symbol too short/long, symbol invalid character, decimals > 18.

**`InstantiateMsg`** is also defined in **`democratized_prime_lib::repo_token`** and re-exported from **`msg.rs`**, so **`pool_v2`**’s `WasmMsg::Instantiate` submessage uses the same serde shape as this contract’s `instantiate` entry point.

---

## 7. References

- [cw20-base contract](https://github.com/CosmWasm/cw-plus/tree/main/contracts/cw20-base)
- [cw20 package (messages and responses)](https://github.com/CosmWasm/cw-plus/tree/main/packages/cw20)
