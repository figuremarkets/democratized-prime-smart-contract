# Pool V2 and Repo Token CW20: Deployment and Config Sequence

This document describes how `**pool_v2**` binds to its receipt token `**repo_token_cw20**`. You can either **reuse an already deployed repo token** or **create a new one in the same transaction** as the pool (the pool sends `WasmMsg::Instantiate` and binds the new address in `reply`).

**Shared Rust types:** The repo token’s on-chain **`InstantiateMsg`** JSON shape and **name / symbol / decimals** validation live in the workspace crate **`democratized-prime-lib`**, module **`repo_token`** (`InstantiateMsg`, `validate_repo_token_meta`). Both **`repo_token_cw20`** (entry `instantiate`) and **`pool_v2`** (when you use `repo_token.new`) use that definition, so the SubMsg payload and the contract’s `instantiate` always stay in sync at compile time.

---

## Two paths at a glance


|                           | **Path A — existing repo token**                                                               | **Path B — new repo token with pool**                                                                              |
| ------------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| **When to use**           | Token created earlier, shared token, or you want full manual control of repo instantiate args. | Greenfield pool: one transaction creates pool + repo token; no separate repo instantiate tx.                       |
| **Repo `InstantiateMsg`** | You send it yourself (Step A1).                                                                | Pool sends it as a submessage: `admin` = pool instantiator, `minter` = pool, `pool_address` = pool.                |
| **UpdateConfig on repo**  | **Required** (Step A3): set `minter` and `pool_address` to the pool.                           | **Not required** for mint/burn or underlying queries; minter and pool are already set.                             |
| Lend                      | Only after Step A3 (pool must be minter).                                                      | As soon as the instantiate transaction succeeds (same block).                                                      |
| **Pool `InstantiateMsg`** | `repo_token`: `{ "existing": { "repo_token_cw20_contract_address": "<cw20>" } }`               | `repo_token`: `{ "new": { "repo_token_code_id", "repo_token_name", "repo_token_symbol", "repo_token_decimals" } }` |


**Contract admin (CosmWasm 2 `WasmMsg::Instantiate::admin`):** On Path B, the pool sets the new repo token’s **contract admin** to the **same address that instantiated the pool** (typically your deployer key), so you can still migrate the CW20. Operational **minter** is always the pool.

---

## Why Path A needs three steps

- **Pool** must know the **CW20 address** (stored as `repo_token_cw20_address`; used to mint when users lend and to burn on withdraw).
- **CW20** must have the **pool** as **minter** so the pool can mint/burn, and `**pool_address`** so Balance/TokenInfo can call `GetReserve` and return **underlying**.
- If the repo token is created **before** the pool exists, you cannot set `minter` / `pool_address` to the pool in that first tx. Hence: instantiate repo with admin as minter → instantiate pool with `repo_token.existing` → **UpdateConfig** on the repo.

Path B avoids that ordering problem: the pool exists first (as the instantiating contract’s address), then the pool’s submessage instantiates the CW20 with `minter` and `pool_address` already pointing at the pool.

---

## Naming convention (multiple pools)

When you have several pools (e.g. HELOC, Auto, Margin) and different **lending tokens** (e.g. YLDS, USD stablecoin, wrapped BTC/ETH), use a consistent pattern so names and symbols are predictable and distinguishable. Replace `{LENDING_TOKEN}` in the patterns below with the actual lending asset (YLDS, USD, ETH, BTC, etc.).

**Pattern:**


| What                                     | Pattern                                      | Meaning                                                                                                                                               |
| ---------------------------------------- | -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Pool** `contract_name` / `description` | `{Product} {Flavor} {LENDING_TOKEN} Pool`    | Product = HELOC, Auto, Margin, etc. Flavor = “Participation Interest”, “Margin”, or omit. LENDING_TOKEN = asset lent/borrowed (e.g. YLDS, USD, ETH).  |
| **Repo token** `name`                    | `{Product} {Flavor} {LENDING_TOKEN} Receipt` | Same product + flavor as the pool; “Receipt” = interest-bearing claim on the pool.                                                                    |
| **Repo token** `symbol`                  | `{productAbbrev}{flavorAbbrev?}{TOKEN}`      | Short ticker: product abbreviation (1–3 letters) + optional flavor letter + lending token ticker (e.g. YLDS, USD, ETH). Use the token's usual ticker. |


**Examples (lending token = YLDS):**


| Product | Flavor                 | Pool description                       | Repo token name                           | Symbol                                    |
| ------- | ---------------------- | -------------------------------------- | ----------------------------------------- | ----------------------------------------- |
| HELOC   | Participation Interest | HELOC Participation Interest YLDS Pool | HELOC Participation Interest YLDS Receipt | **HPYLDS** (HELOC + Participation + YLDS) |
| Auto    | Participation Interest | Auto Participation Interest YLDS Pool  | Auto Participation Interest YLDS Receipt  | **APYLDS** (Auto + Participation + YLDS)  |
| Margin  | —                      | Margin YLDS Pool                       | Margin YLDS Receipt                       | **MYLDS** (Margin + YLDS)                 |


Symbols are conventionally **all caps** (e.g. HPYLDS, PUSD, METH). For other lending tokens use the same pattern (e.g. HPUSD, METH). If two products would collide (e.g. “Auto” and “Asset” → AUSD), add another letter (**AUUSD** vs **ASUSD**) or spell the product more (**AUTOUSD**).

---

## Path A: Existing repo token (three steps)

### Step A1: Instantiate the repo token CW20

**Contract:** `repo_token_cw20`

**Message:** `InstantiateMsg`


| Field          | Value / meaning                                                                                                                                                           |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`         | Token name: `{Product} {Flavor} {LENDING_TOKEN} Receipt` (e.g. `"HELOC Participation Interest YLDS Receipt"` or `"Margin USD Receipt"`).                                  |
| `symbol`       | Short ticker, conventionally all caps: product/flavor abbreviation + lending token (e.g. `"HPYLDS"`, `"PUSD"`, `"METH"`). Match the lending asset (YLDS, USD, ETH, etc.). |
| `decimals`     | Same as the pool’s lending denom (e.g. `6` for USD/YLDS-style, `8` for BTC-style, `18` for ETH-style).                                                                    |
| `admin`        | Address that may call `UpdateConfig` (e.g. deployer or multisig).                                                                                                         |
| `minter`       | **Use the same as `admin`** for now. The pool will become minter in Step A3.                                                                                              |
| `pool_address` | `**None**` (or omit). The pool does not exist yet.                                                                                                                        |


**Outcome:** The CW20 contract is on-chain. Only `admin` can mint/burn until you change minter. Balance/TokenInfo return **scaled** amounts until `pool_address` is set.

---

### Step A2: Instantiate the pool (`pool_v2`)

**Contract:** `pool_v2`

**Message:** `InstantiateMsg` — include `**repo_token`** with the `**existing**` variant (not the legacy flat field; there is no top-level `repo_token_cw20_address`).


| Field           | Value / meaning                                                                                                                                                                                                                                                                    |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `repo_token`    | `{ "existing": { "repo_token_cw20_contract_address": "<CW20 from Step A1>" } }`                                                                                                                                                                                                    |
| `lending_denom` | Lending denom and precision.                                                                                                                                                                                                                                                       |
| `rate_params`   | Kink model parameters.                                                                                                                                                                                                                                                             |
| (other fields)  | Oracle, collateral, margin/liquidation, etc. **`lender_required_attrs` / `borrower_required_attrs`:** list of attribute names; sender must have **all** of these (empty list = no check). Can be updated later by admin via **SetLenderRequiredAttrs** / **SetBorrowerRequiredAttrs**. |


**Outcome:** The pool is on-chain and stores the CW20 address. **Do not call Lend yet:** the CW20’s minter is still the admin, so the pool is not allowed to mint.

---

### Step A3: Update the repo token config

**Contract:** `repo_token_cw20` (same as Step A1)

**Message:** `ExecuteMsg::UpdateConfig`


| Field          | Value / meaning                                                         |
| -------------- | ----------------------------------------------------------------------- |
| `minter`       | `**Some(pool_address)`** — the `pool_v2` contract address from Step A2. |
| `pool_address` | `**Some(pool_address)**` — same pool address.                           |


**Sender:** Must be the CW20 **admin** (the address set in Step A1).

**Outcome:** The pool is the **minter**; Balance/TokenInfo return **underlying** once `pool_address` is set. After this, the system is fully operational.

---

## Path B: New repo token in the pool instantiate transaction

### Single step: Instantiate the pool with `repo_token.new`

**Contract:** `pool_v2`

**Message:** `InstantiateMsg` with:


| Field        | Value / meaning                                                                                                                 |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| `repo_token` | `{ "new": { "repo_token_code_id": <u64>, "repo_token_name": "...", "repo_token_symbol": "...", "repo_token_decimals": <u8> } }` |


The pool validates name/symbol/decimals via the same **`democratized_prime_lib::repo_token::validate_repo_token_meta`** used inside `repo_token_cw20`’s `instantiate`, then schedules a `**SubMsg`** that instantiates `repo_token_cw20` with:

- `admin` = address that instantiated the pool (`info.sender`),
- `minter` = pool contract address,
- `pool_address` = pool contract address,

so **no `UpdateConfig` is required** for normal operation.

**Outcome:** In one transaction, after `reply` succeeds, `repo_token_cw20_address` is set on the pool and the new CW20 is fully wired. You can lend in a follow-up transaction immediately.

**Discovering the new repo address:** Query the pool with `**get_state`**. In the JSON response, the repo CW20 address is on the contract state as `**atca**` (short serde key for `repo_token_cw20_address`). You can also read it from instantiate events.

**Requirements:** The `**repo_token_code_id`** must be the uploaded `repo_token_cw20` code ID on your chain. The pool’s `WasmMsg::Instantiate` uses `**admin: Some(instantiator)**` on the submessage so your deployer remains contract admin for migrations.

---

## Summary diagrams

### Path A (existing repo token)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ A1: Instantiate repo_token_cw20                                         │
│     admin = deployer, minter = deployer, pool_address = None             │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ A2: Instantiate pool_v2                                                 │
│     repo_token.existing.repo_token_cw20_contract_address = <CW20>        │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ A3: Execute UpdateConfig on repo_token_cw20 (as admin)                  │
│     minter = <pool>, pool_address = <pool>                              │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
                    Pool can mint/burn; CW20 shows underlying.
```

### Path B (new repo token with pool)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Instantiate pool_v2 with repo_token.new (code_id, name, symbol, dec.) │
│   → SubMsg: WasmMsg::Instantiate repo_token_cw20                        │
│        (admin = instantiator, minter = pool, pool_address = pool)       │
│   → reply: bind repo_token_cw20_address on pool                          │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
              One tx: pool + repo token ready; lending allowed next tx.
```

---

## CLI examples

Use your chain’s CosmWasm CLI (`provenanced`, `wasmd`, `junod`, etc.). Replace placeholders and code IDs as needed. Sender must have enough gas and (for instantiate) the chain’s upload/instantiate permissions.

**Placeholders:**


| Placeholder       | Meaning                                               |
| ----------------- | ----------------------------------------------------- |
| `$CLI`            | Binary name (e.g. `provenanced` or `wasmd`).          |
| `$CHAIN_ID`       | Chain ID (e.g. `chain-local`).                        |
| `$KEY_NAME`       | Key name in your keyring for the signer.              |
| `$ADMIN`          | Bech32 address of the admin/deployer (e.g. `tp1...`). |
| `$CODE_ID_CW20`   | Code ID of the uploaded `repo_token_cw20` contract.   |
| `$CODE_ID_POOL`   | Code ID of the uploaded `pool_v2` contract.           |
| `$CW20_ADDRESS`   | Contract address returned after Path A Step A1.       |
| `$POOL_ADDRESS`   | Contract address returned after pool instantiation.   |
| `$ORACLE_ADDRESS` | Bech32 address of the price oracle contract.          |


---

### Path A — Step A1: Instantiate `repo_token_cw20`

**Instantiate message (JSON):** admin and minter set to the deployer; no pool yet. Repo token name/symbol should identify the pool and **lending token** (e.g. YLDS, USD, ETH—whatever asset the pool lends). Example below uses YLDS; use the same pattern for USD, ETH, etc.

```json
{
  "name": "HELOC Participation Interest YLDS Receipt",
  "symbol": "HPYLDS",
  "decimals": 6,
  "admin": "$ADMIN",
  "minter": "$ADMIN",
  "pool_address": null
}
```

**CLI (inline JSON):**

```bash
$CLI tx wasm instantiate $CODE_ID_CW20 \
  '{"name":"HELOC Participation Interest YLDS Receipt","symbol":"HPYLDS","decimals":6,"admin":"'$ADMIN'","minter":"'$ADMIN'","pool_address":null}' \
  --label "heloc-participation-ylds-receipt" \
  --from $KEY_NAME \
  --chain-id $CHAIN_ID \
  --gas auto --gas-adjustment 1.3 \
  --gas-prices <your-gas-price> \
  -y
```

From the transaction result, note the **contract address** of the new CW20 → `$CW20_ADDRESS` for Step A2.

---

### Path A — Step A2: Instantiate `pool_v2`

**Instantiate message (JSON):** use `repo_token.existing` with the CW20 address from Step A1. **Lending token/denom** can be anything (YLDS, USD, wrapped BTC/ETH, etc.); set `lending_denom` to that asset’s denom and precision. Example below uses YLDS (`uylds.fcc`, 6 decimals). The pool’s message uses shortened field names for some nested types (e.g. lending denom `"n"` / `"p"`, rate params `"tr"`, `"minr"`, etc.).

```json
{
  "contract_name": "HELOC Participation Interest YLDS Pool",
  "description": "HELOC Participation Interest YLDS Pool",
  "repo_token": {
    "existing": {
      "repo_token_cw20_contract_address": "$CW20_ADDRESS"
    }
  },
  "lending_denom": { "n": "uylds.fcc", "p": 6 },
  "rate_params": {
    "tr": "0.09",
    "minr": "0.0325",
    "maxr": "0.20",
    "kink": "0.90",
    "rf": "0.005",
    "spy": 31536000
  },
  "lender_required_attrs": [],
  "borrower_required_attrs": [],
  "price_oracle_address": "$ORACLE_ADDRESS",
  "max_borrower_collateral_types": 5,
  "margin_rate": "0.80",
  "liquidation_rate": "0.90",
  "liquidation_bonus_rate": "1.02",
  "min_lend": "1",
  "min_borrow": "1",
  "supported_collateral_assets": [
    { "id": "nbtc.figure.se", "h": "0.80" }
  ]
}
```

**CLI (inline JSON):** adjust `repo_token` and fields as needed.

---

### Path A — Step A3: `UpdateConfig` on `repo_token_cw20`

Same as before: set `minter` and `pool_address` to the pool. Only the CW20 **admin** may send this.

```json
{
  "update_config": {
    "minter": "$POOL_ADDRESS",
    "pool_address": "$POOL_ADDRESS"
  }
}
```

```bash
$CLI tx wasm execute $CW20_ADDRESS \
  '{"update_config":{"minter":"'$POOL_ADDRESS'","pool_address":"'$POOL_ADDRESS'"}}' \
  --from $KEY_NAME \
  --chain-id $CHAIN_ID \
  --gas auto --gas-adjustment 1.3 \
  -y
```

After this succeeds, the pool can mint/burn the repo token and the CW20’s Balance/TokenInfo return underlying amounts.

---

### Path B — Instantiate pool with embedded repo token

**Instantiate message (JSON):** replace `repo_token` with the `**new`** variant. `repo_token_code_id` must match `$CODE_ID_CW20` on your chain.

```json
{
  "contract_name": "HELOC Participation Interest YLDS Pool",
  "description": "HELOC Participation Interest YLDS Pool",
  "repo_token": {
    "new": {
      "repo_token_code_id": 123,
      "repo_token_name": "HELOC Participation Interest YLDS Receipt",
      "repo_token_symbol": "HPYLDS",
      "repo_token_decimals": 6
    }
  },
  "lending_denom": { "n": "uylds.fcc", "p": 6 },
  "rate_params": {
    "tr": "0.09",
    "minr": "0.0325",
    "maxr": "0.20",
    "kink": "0.90",
    "rf": "0.005",
    "spy": 31536000
  },
  "lender_required_attrs": [],
  "borrower_required_attrs": [],
  "price_oracle_address": "$ORACLE_ADDRESS",
  "max_borrower_collateral_types": 5,
  "margin_rate": "0.80",
  "liquidation_rate": "0.90",
  "liquidation_bonus_rate": "1.02",
  "min_lend": "1",
  "min_borrow": "1",
  "supported_collateral_assets": [
    { "id": "nbtc.figure.se", "h": "0.80" }
  ]
}
```

Use `**123**` only as a placeholder — substitute your real `$CODE_ID_CW20`.

**CLI:** pass the full JSON to `tx wasm instantiate` for `$CODE_ID_POOL` (same flags as Path A). After the transaction, query `**get_state`** on the pool to obtain the new repo token address.

**Optional `--admin` on the pool instantiate:** If your chain sets a contract admin on the **pool**, use the same policy as for any wasm contract; the repo token’s **contract admin** is set from the pool instantiator, not from the pool’s migrate admin field.

---

## What happens if you skip or reorder?

**Path A**

- **Lend before Step A3:** The pool will try to mint on the CW20. The CW20 will reject it (only minter may mint). Complete **UpdateConfig** before lending.
- **Setting pool as minter before the pool exists:** You do not have the pool address yet, so you cannot set it when the CW20 is created alone. That is why Path A uses CW20 → pool → UpdateConfig.
- **Never setting `pool_address`:** Transfer and Send fail with `PoolNotConfigured` until it is set. Balance and TokenInfo return **scaled** amounts instead of underlying. Mint/burn by the minter still work.

**Path B**

- If the `**SubMsg`** fails (e.g. wrong `repo_token_code_id`), the whole pool instantiation fails and no pool is created.
- `**repo_token_code_id` = 0** is rejected by the pool.

---

## References

- Repo token contract: `contracts/repo_token_cw20/` (see its README). **`InstantiateMsg`** is re-exported from **`packages/lib`** (`democratized_prime_lib::repo_token`).
- Pool contract: `contracts/pool_v2/` (see CODEBASE_WALKTHROUGH.md; pool **`InstantiateMsg`** is in `src/msg/instantiate.rs`; repo CW20 SubMsg uses the shared **`InstantiateMsg`** above). Authoritative **`ExecuteMsg` / `QueryMsg` / `InstantiateMsg`** JSON is in `contracts/pool_v2/schema/`; **`MigrateMsg`** is empty (`{}`).

