# CLI test playbook: pool_v2 + repo_token_cw20 (one pool)

This doc gives **copy-pastable CLI commands** to deploy and then **exercise every endpoint** of:

- **One pool_v2** with its **repo_token_cw20** (example: HELOC YLDS pool)

One pool is enough to show how pool_v2 works; multiple pools are only needed when using a vault wrapper (meta_pool) to aggregate them.

**Assumptions:** You have a running chain (e.g. Provenance localnet), a key with funds (for fees and lending denom), and the WASM for `repo_token_cw20`, `pool_v2`, and `price_oracle`. You must have a **price oracle** address for pool_v2 (or a mock) and the lending denom (e.g. `uylds.fcc`) created/funded as needed.

---

## 1. Prerequisites and placeholders

Set these once and use them in all commands below. Replace with your chain-id, key, and addresses. All commands use the `provenanced` binary. **Keyring:** the doc uses `--keyring-backend test --testnet` so `admin`, `lender`, and `borrower` are resolved from the test keyring; for localnet add `--home build/node0` (or your node home) if your keys live there.

| Variable | Meaning |
|----------|--------|
| `$CHAIN_ID` | Chain ID (e.g. `chain-local`) |
| `$ADMIN` | Bech32 address of key **admin** (deployer, pool admin) |
| `$LENDER` | Bech32 address of key **lender** (lends into pools) |
| `$BORROWER` | Bech32 address of key **borrower** (borrows, adds collateral, repays) |
| `$LENDING_DENOM` | Lending token denom (e.g. `uylds.fcc`) |
| `$COLLATERAL_DENOM` | Pool collateral denom (e.g. `nheloc.figure.pm`); must match pool `supported_collateral_assets` |
| `$ORACLE_ADDRESS` | Price oracle contract address (set after instantiating price_oracle in section 2) |
| `$CODE_ID_REPO` | Code ID of uploaded `repo_token_cw20` |
| `$CODE_ID_POOL` | Code ID of uploaded `pool_v2` |
| `$CODE_ID_ORACLE` | Code ID of uploaded `price_oracle` |

Gas/fees: use **`--gas-prices 1nhash --gas-adjustment 2`**; gas is estimated automatically. Adjust for your network's minimums.

**Addresses you will fill after deployment:**

| Variable | Meaning |
|----------|--------|
| `$REPO` | Pool repo token CW20 address |
| `$POOL` | pool_v2 contract address |

**Optional:** For lender KYC, a KYC attribute name (e.g. `lender.kyc.pb`) — see section 2a.

### Create keys: admin, lender, lender2, borrower

Create four keyring keys so you can test as distinct admin, lender, a second lender (for Transfer recipient; pool requires recipient to have lender attr), and borrower. Use the same keyring backend and testnet flag as the rest of your setup (e.g. `--keyring-backend test --testnet`). For localnet you may need `--home build/node0` (or your node home).

```bash
provenanced keys add admin --keyring-backend test --testnet
provenanced keys add lender --keyring-backend test --testnet
provenanced keys add lender2 --keyring-backend test --testnet
provenanced keys add borrower --keyring-backend test --testnet
```

Then set address variables from the key names (no variables to inject — use these literal key names everywhere). **Localnet** uses Bech32 prefix **pb**; the **--testnet** flag makes `keys show` use prefix **tp**, which causes "invalid Bech32 prefix; expected pb, got tp" on name bind and other txs. So for localnet, omit **--testnet** when setting address variables below:

```bash
# Amount per account (e.g. 100B nhash); adjust as needed. On localnet, node0 usually holds the initial supply.
export CHAIN_ID=chain-local
# For localnet (pb prefix): omit --testnet. For testnet (tp prefix): add --testnet.
export ADMIN=$(provenanced keys show -a admin --keyring-backend test --testnet)
export LENDER=$(provenanced keys show -a lender --keyring-backend test --testnet)
export LENDER2=$(provenanced keys show -a lender2 --keyring-backend test --testnet)
export BORROWER=$(provenanced keys show -a borrower --keyring-backend test --testnet)
export LENDING_DENOM=uylds.fcc
export COLLATERAL_DENOM=nheloc.figure.pm
export CODE_ID_REPO=1
export CODE_ID_POOL=2
export CODE_ID_ORACLE=3
# ORACLE_ADDRESS is set after instantiating the price oracle (section 2)
```

### Fund admin, lender, lender2, and borrower with nhash

Each account needs nhash to pay transaction fees. From an account that already has nhash (e.g. **node0** on localnet), send to admin, lender, lender2, and borrower. `bank send` takes sender (key name or address), then recipient address, then amount. Run from the directory where `build/node0` exists. For testnet or other networks, omit `--home build/node0` and use your faucet key name.

```bash
export FAUCET_KEY=node0
export NHASH_PER_ACCOUNT=100000000000nhash

provenanced --home build/node0 tx bank send $FAUCET_KEY $ADMIN $NHASH_PER_ACCOUNT \
  --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y

provenanced --home build/node0 tx bank send $FAUCET_KEY $LENDER $NHASH_PER_ACCOUNT \
  --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y

provenanced --home build/node0 tx bank send $FAUCET_KEY $LENDER2 $NHASH_PER_ACCOUNT \
  --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y

provenanced --home build/node0 tx bank send $FAUCET_KEY $BORROWER $NHASH_PER_ACCOUNT \
  --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

After marker creation (section 2a), **$LENDING_DENOM** is minted to the lender (to lend into the pool) and **$COLLATERAL_DENOM** to the borrower (for AddCollateral/Borrow). No separate send needed if you follow the doc.

---

## 2. Package WASM (build optimized artifacts)

From the repo root, build and optimize the contracts. This uses the CosmWasm optimizer (Docker or Podman) and writes into `artifacts/`:

```bash
make optimize
```

Requires Docker or Podman. Outputs (in `artifacts/`): `repo_token_cw20.wasm`, `democratized_prime_pool_v2.wasm`, `democratized_prime_price_oracle.wasm`. Use these paths in the store commands below.

---

## 2b. Upload WASM (if not already uploaded)

```bash
# From repo root; paths match make optimize output. Use test keyring so "admin" is found.
# Gas is auto-estimated; --gas-adjustment 2 gives headroom for store.
provenanced tx wasm store artifacts/repo_token_cw20.wasm --from admin --chain-id $CHAIN_ID --gas auto --gas-prices 1nhash --gas-adjustment 2 --keyring-backend test --testnet -b sync -y
provenanced tx wasm store artifacts/democratized_prime_pool_v2.wasm --from admin --chain-id $CHAIN_ID --gas auto --gas-prices 1nhash --gas-adjustment 2 --keyring-backend test --testnet -b sync -y
provenanced tx wasm store artifacts/democratized_prime_price_oracle.wasm --from admin --chain-id $CHAIN_ID --gas auto --gas-prices 1nhash --gas-adjustment 2 --keyring-backend test --testnet -b sync -y
```

From each tx result, note the **code_id** and set `$CODE_ID_REPO`, `$CODE_ID_POOL`, and `$CODE_ID_ORACLE` accordingly.

**Tx status (when indexing is enabled):** Store returns a `txhash` immediately; the tx may still fail in the next block. To check status and get the **code_id**:

```bash
provenanced q tx TXHASH --chain-id $CHAIN_ID --testnet -o json
```

Replace `TXHASH` with the hash from the store response. `code: 0` = success; non-zero = failure (see `raw_log`). On success, **code_id** is in the tx events.

If you see **"transaction indexing is disabled"**, the node isn't indexing txs so `q tx` by hash isn't available. To **enable tx indexing on localnet**: in the node home (e.g. `build/node0`), edit `config/config.toml` and set `indexer = "kv"` under `[tx_index]` (replace `indexer = "null"` if present). Restart the node so the change takes effect; then `q tx TXHASH` will work. Alternatively, use this to confirm the code is stored and get code IDs:

```bash
provenanced q wasm list-code --chain-id $CHAIN_ID --testnet -o json
```

Run it a few seconds after the store; if your store succeeded, the new code will appear there.

### Instantiate price oracle

pool_v2 requires a price oracle address. Instantiate the `price_oracle` contract (admin can update asset mappings and set prices for collateral valuation):

```bash
provenanced tx wasm instantiate $CODE_ID_ORACLE '{"admin":"'$ADMIN'"}' \
  --label "price-oracle" \
  --admin $ADMIN \
  --from admin --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

**Get the contract address:** If the CLI doesn't show it in the instantiate output, query by code ID (latest contract is last in the list):

```bash
provenanced q wasm list-contract-by-code $CODE_ID_ORACLE --chain-id $CHAIN_ID --testnet -o json
# Set the address (e.g. last in list):
export ORACLE_ADDRESS=$(provenanced q wasm list-contract-by-code $CODE_ID_ORACLE --chain-id $CHAIN_ID --testnet -o json | jq -r '.contracts[-1]')
```

Use **$ORACLE_ADDRESS** in pool_v2 instantiate (section 3). To support Borrow/AddCollateral you will need to set prices for your collateral assets (e.g. via the price_oracle contract's execute messages); see `contracts/price_oracle/` for the API.

---

## 2a. Provenance: create uylds.fcc marker and KYC attribute (optional)

If you are on **Provenance** and need the lending denom as a marker plus a lender KYC requirement, do the following before deployment. Omit if your chain already has `uylds.fcc` or you use another denom source. For **localnet**, add `--home build/node0` (or your node home) to commands if your keyring/node use it.

**Order:** Only the **lending** marker is restricted (uses `--required-attributes=lender.kyc.pb`). Do the **KYC** steps first (bind names, add `lender.kyc.pb` to lender), then create the lending marker and mint to lender. Collateral marker is unrestricted (COIN); create it and mint to borrower. After deployment, grant `lender.kyc.pb` to the pool contract and borrower (section 3.2).

### Create the uylds.fcc marker

Create, finalize, and activate a **restricted** marker so the admin can have mint, withdraw, and transfer (plain COIN markers do not support ACCESS_TRANSFER). Initial supply is 0; you mint in the next step. The second argument is the access grant: **address, then permissions** — so `"$ADMIN,mint,admin,withdraw,transfer"` means "grant `$ADMIN` the roles mint, admin, withdraw, transfer". That is correct: `$ADMIN` is the address that receives those permissions.

```bash
# Create + finalize + activate as RESTRICTED; require lender.kyc.pb to receive
provenanced tx marker create-finalize-activate \
  0$LENDING_DENOM \
  "$ADMIN,mint,admin,withdraw,transfer" \
  --type RESTRICTED \
  --required-attributes=lender.kyc.pb \
  --supplyFixed=false \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y
```

**Note:** The lender (and later the pool contracts and borrower) must have the `lender.kyc.pb` attribute to receive this marker. Do the "KYC attribute for lenders" steps first so the lender has it before minting; after deployment, grant it to the pool contract and borrower (see "After deployment: grant lender.kyc.pb" in section 3.2).

Mint the lending denom to the **lender** so they can lend into pool_v2. Amounts are in **base units** (6 decimals → 1000000000 = 1000 YLDS). Optionally mint to admin as well if needed.

```bash
provenanced tx marker mint 1000000000$LENDING_DENOM $LENDER \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y
```

**Query the marker:** By denom (your lending denom, e.g. `uylds.fcc`), or list all markers:

```bash
# Details for one marker (use your $LENDING_DENOM, e.g. uylds.fcc)
provenanced q marker get $LENDING_DENOM --chain-id $CHAIN_ID --testnet -o json

# List all markers on the chain
provenanced q marker list --chain-id $CHAIN_ID --testnet -o json
```

### Create collateral marker

The pool's **supported_collateral_assets** lists which denoms can be used as collateral. Collateral markers are **unrestricted (COIN)** — no required attributes. Create one marker and mint to the borrower.

```bash
provenanced tx marker create-finalize-activate \
  0$COLLATERAL_DENOM \
  "$ADMIN,mint,admin,withdraw" \
  --supplyFixed=false \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y

provenanced tx marker mint 1000000000$COLLATERAL_DENOM $BORROWER \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y
```

Set a price for the collateral in the price oracle so the pool can value it for Borrow/Liquidate. The oracle admin (e.g. **admin**) must send **UpdateAssetPrices** with USD prices for the **lending denom** and **collateral denom**. Use realistic USD values; omit `as_of` to use block time.

```bash
# Lending denom (e.g. 1 for a stablecoin) and collateral. Adjust "usd" values as needed.
provenanced tx wasm execute $ORACLE_ADDRESS \
  '{"update_asset_prices":{"prices":[{"asset":"'$LENDING_DENOM'","usd":"1"},{"asset":"'$COLLATERAL_DENOM'","usd":"1.5"}]}}' \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

### KYC attribute for lenders and borrowers

Provenance uses **name bindings** and **attributes**. pool_v2 can require that lenders (and Transfer recipients) have a given attribute (e.g. `lender.kyc.pb`). The contract only checks that the attribute **exists** on the account; value/type do not matter.

When the **lending marker** is restricted with `--required-attributes=lender.kyc.pb`, any account that **receives** the lending denom (including the **borrower** on Borrow) must have that attribute at the chain level. For consistency, set **`borrower_required_attrs`** to the same list (e.g. `["lender.kyc.pb"]`) in pool_v2 so only KYC'd accounts can Borrow, AddCollateral, and RemoveCollateral — and grant the attribute to the borrower (see section 3.2).

**1. Bind the unrestricted base name `kyc.pb`** (once per chain). Use the key that owns the **pb** root (e.g. **node0** on localnet) as `--from`:

```bash
provenanced tx name bind \
  "kyc" \
  $ADMIN \
  "pb" \
  --unrestrict \
  --from node0 --home build/node0 \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y
```

**2. Bind the restricted name `lender.kyc.pb`** (owner is the KYC authority; e.g. admin):

```bash
provenanced tx name bind \
  "lender" \
  $ADMIN \
  "kyc.pb" \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y
```

**3. Add the `lender.kyc.pb` attribute to the lender account** (so they can lend). Use the KYC authority key as `--from`; the account that receives the attribute is the one that will lend into the pool:

```bash
# Grant lender.kyc.pb to the lender (so they can lend). Use the key that owns "lender.kyc.pb" as --from.
provenanced tx attribute add \
  "lender.kyc.pb" \
  $LENDER \
  "string" \
  "ok" \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y
```

**3b. Add the `lender.kyc.pb` attribute to the borrower** (so they can receive YLDS on Borrow — the restricted marker requires it — and so the pool's **`borrower_required_attrs`** check passes). Use the same `--from` (KYC authority, e.g. admin):

```bash
provenanced tx attribute add \
  "lender.kyc.pb" \
  $BORROWER \
  "string" \
  "ok" \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y
```

For the second lender (**lender2**), add the same attribute so they can receive repo token Transfers (pool requires the recipient to have the lender attribute). Use the same `--from` (KYC authority, e.g. admin):

```bash
provenanced tx attribute add \
  "lender.kyc.pb" \
  $LENDER2 \
  "string" \
  "ok" \
  --from admin \
  --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 \
  -y
```

---

## 3. Deployment

Choose **one** path. Path A matches the older “repo first, then pool, then UpdateConfig” flow. Path B creates the repo token inside **pool_v2** instantiate (`repo_token.new`); skip repo instantiate and **skip UpdateConfig** on the repo.

### 3.1 Path A: existing repo token → pool → UpdateConfig

**3.1.1 Instantiate repo token**

```bash
provenanced tx wasm instantiate $CODE_ID_REPO \
  '{"name":"HELOC Participation Interest YLDS Receipt","symbol":"HPYLDS","decimals":6,"admin":"'$ADMIN'","minter":"'$ADMIN'","pool_address":null}' \
  --label "pool-repo-token" \
  --admin $ADMIN \
  --from admin --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

Set **$REPO** from the result, or: `export REPO=$(provenanced q wasm list-contract-by-code $CODE_ID_REPO --chain-id $CHAIN_ID --testnet -o json | jq -r '.contracts[-1]')`

**3.1.2 Instantiate pool_v2** (`repo_token.existing`)

```bash
POOL_MSG=$(cat <<EOF
{
  "contract_name": "HELOC Participation Interest YLDS Pool",
  "description": "HELOC YLDS Pool",
  "repo_token": {
    "existing": {
      "repo_token_cw20_contract_address": "$REPO"
    }
  },
  "lending_denom": { "n": "$LENDING_DENOM", "p": 6 },
  "rate_params": { "tr": "0.09", "minr": "0.0325", "maxr": "0.20", "kink": "0.90", "rf": "0.005", "fm": "reserve_factor", "ff": "0", "spy": 31536000 },
  "lender_required_attrs": ["lender.kyc.pb"],
  "borrower_required_attrs": ["lender.kyc.pb"],
  "price_oracle_address": "$ORACLE_ADDRESS",
  "max_borrower_collateral_types": 5,
  "margin_rate": "0.80",
  "liquidation_rate": "0.90",
  "liquidation_bonus_rate": "1.02",
  "min_lend": "1",
  "min_borrow": "1",
  "supported_collateral_assets": [ { "id": "$COLLATERAL_DENOM", "h": "0.80" } ],
  "commit_market_id": null
}
EOF
)
provenanced tx wasm instantiate $CODE_ID_POOL "$POOL_MSG" \
  --label "pool-v2" \
  --admin $ADMIN \
  --from admin --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

Set **$POOL** from the result, or: `export POOL=$(provenanced q wasm list-contract-by-code $CODE_ID_POOL --chain-id $CHAIN_ID --testnet -o json | jq -r '.contracts[-1]')`

Optional: **commit_market_id** can be set at instantiate (e.g. `"commit_market_id": 1` for a Provenance exchange market) so that user withdraws with **commit_funds: true** emit MsgCommitFundsRequest. Omit or set to `null` if not using commit-on-exit.

### Rate params fee model (`rate_params.fm`, `rate_params.ff`)

`pool_v2` supports two protocol-fee modes inside `rate_params`:

- `fm: "reserve_factor"` (legacy): lender rate uses `rf`; set `ff` to `"0"`.
- `fm: "flat_borrow_spread"` (new): protocol takes fixed annual spread `ff` from borrower APR.

Validation rules:

- In `reserve_factor` mode, `ff` must be zero.
- In `flat_borrow_spread` mode, `ff <= minr` (prevents negative lender rate at low utilization).

Example `rate_params` for flat spread mode:

```json
{
  "tr": "0.09",
  "minr": "0.0325",
  "maxr": "0.20",
  "kink": "0.90",
  "rf": "0.005",
  "fm": "flat_borrow_spread",
  "ff": "0.005",
  "spy": 31536000
}
```

**3.1.3 UpdateConfig on repo (minter + pool_address = pool)**

Required on Path A so the pool can mint/burn and the CW20 can show underlying balances.

```bash
provenanced tx wasm execute $REPO \
  '{"update_config":{"minter":"'$POOL'","pool_address":"'$POOL'"}}' \
  --from admin --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

---

### 3.2 Path B: pool instantiate with `repo_token.new` (no separate repo tx, no UpdateConfig)

One transaction: **pool_v2** instantiates `repo_token_cw20` as a submessage with `minter` and `pool_address` already set to the pool. Set **`repo_token_code_id`** to **`$CODE_ID_REPO`** (the same code ID you would use to upload `repo_token_cw20`).

```bash
POOL_MSG=$(cat <<EOF
{
  "contract_name": "HELOC Participation Interest YLDS Pool",
  "description": "HELOC YLDS Pool",
  "repo_token": {
    "new": {
      "repo_token_code_id": $CODE_ID_REPO,
      "repo_token_name": "HELOC Participation Interest YLDS Receipt",
      "repo_token_symbol": "HPYLDS",
      "repo_token_decimals": 6
    }
  },
  "lending_denom": { "n": "$LENDING_DENOM", "p": 6 },
  "rate_params": { "tr": "0.09", "minr": "0.0325", "maxr": "0.20", "kink": "0.90", "rf": "0.005", "fm": "reserve_factor", "ff": "0", "spy": 31536000 },
  "lender_required_attrs": ["lender.kyc.pb"],
  "borrower_required_attrs": ["lender.kyc.pb"],
  "price_oracle_address": "$ORACLE_ADDRESS",
  "max_borrower_collateral_types": 5,
  "margin_rate": "0.80",
  "liquidation_rate": "0.90",
  "liquidation_bonus_rate": "1.02",
  "min_lend": "1",
  "min_borrow": "1",
  "supported_collateral_assets": [ { "id": "$COLLATERAL_DENOM", "h": "0.80" } ]
}
EOF
)
provenanced tx wasm instantiate $CODE_ID_POOL "$POOL_MSG" \
  --label "pool-v2-with-repo" \
  --admin $ADMIN \
  --from admin --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

Set **$POOL** as above. Set **$REPO** by querying the pool’s **GetState** response: the repo CW20 address is JSON field **`atca`** on the embedded `contract` object (short serde key for `repo_token_cw20_address`). Example:

`export REPO=$(provenanced query wasm contract-state smart $POOL '{"get_state":{}}' --chain-id $CHAIN_ID --testnet -o json | jq -r '.data.contract.atca')`

If your CLI nests or base64-encodes `data`, adjust the `jq` path accordingly.

---

### 3.3 After deployment: grant lender.kyc.pb to contracts and borrower (for restricted lending marker)

The **lending** marker is restricted and requires `lender.kyc.pb` on any account that **receives** it. Addresses that receive the lending denom: **$POOL** (lend), **$BORROWER** (on Borrow), and **$ADMIN** (on **WithdrawReserve** when recipient is omitted — reserve is sent to admin).

Run each grant separately so the previous tx is committed before the next (avoids account sequence errors when sending from the same key in quick succession):

```bash
provenanced tx attribute add "lender.kyc.pb" $ADMIN "string" "ok" \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y

provenanced tx attribute add "lender.kyc.pb" $POOL "string" "ok" \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y

provenanced tx attribute add "lender.kyc.pb" $BORROWER "string" "ok" \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

---

## 4. Test pool_v2 (broadcast then verify in sequence)

Use **$POOL** and **$REPO**. Fund **lender** with **$LENDING_DENOM**; fund **borrower** with **$COLLATERAL_DENOM** and **$LENDING_DENOM** for Repay. Ensure the price oracle has prices for the collateral denom. Run each step in order; after each broadcast, run the suggested query to confirm state.

**4.1 Baseline state** — before anyone lends, check pool, reserve, and (optional) repo token config:

```bash
provenanced query wasm contract-state smart $POOL '{"get_state":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $POOL '{"get_reserve":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $REPO '{"token_info":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $REPO '{"minter":{}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.2 Lend** (lender) → then verify liquidity and lender's repo balance. Use a meaningful amount so the pool has liquidity for Borrow/Withdraw (e.g. 100 YLDS = 100000000 with 6 decimals). `min_lend` in pool config is the minimum per tx (e.g. 1), not the cumulative amount lent.

```bash
provenanced tx wasm execute $POOL '{"lend":{}}' \
  --from lender --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --amount 100000000$LENDING_DENOM \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

```bash
provenanced query wasm contract-state smart $POOL '{"get_state":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $POOL '{"get_reserve":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $REPO '{"balance":{"address":"'$LENDER'"}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.3 Withdraw** (lender; via CW20 Send to pool with withdraw payload) → then verify state and balance decreased. Amounts in base units (e.g. 10000000 = 10 YLDS with 6 decimals). If the lender has **require_commit_on_exit** set (see 4.11b), the payload must include **"commit_funds": true** and the pool must have **commit_market_id** set; the chain will then receive MsgCommitFundsRequest on withdraw.

```bash
MSG_B64=$(echo -n '{"withdraw":{"amount":"10000000"}}' | base64)
provenanced tx wasm execute $REPO '{"send":{"contract":"'$POOL'","amount":"10000000","msg":"'$MSG_B64'"}}' \
  --from lender --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

```bash
provenanced query wasm contract-state smart $POOL '{"get_state":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $REPO '{"balance":{"address":"'$LENDER'"}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.4 Optional: WithdrawExact, Transfer (via pool), TransferExact** — same pattern: broadcast then query GetState / GetReserve / balance as needed. Repo token only allows Transfer/Send to the pool; user-to-user moves go through the pool (Send to pool with transfer payload; recipient must have lender attr). **Note:** If the *sender* has **require_commit_on_exit** set, Transfer and TransferExact are **not allowed** (they must withdraw with commit_funds first).

```bash
# WithdrawExact (replace SHARES_AMOUNT with actual scaled amount)
MSG_B64=$(echo -n '{"withdraw_exact":{}}' | base64)
provenanced tx wasm execute $REPO '{"send":{"contract":"'$POOL'","amount":"SHARES_AMOUNT","msg":"'$MSG_B64'"}}' \
  --from lender --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
# Transfer underlying to another lender via pool (recipient must have lender attr)
MSG_B64=$(echo -n '{"transfer":{"recipient":"'$LENDER2'","amount":"1000000"}}' | base64)
provenanced tx wasm execute $REPO '{"send":{"contract":"'$POOL'","amount":"1000000","msg":"'$MSG_B64'"}}' \
  --from lender --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
# Then e.g. query GetState, $REPO balance for lender / $LENDER2
```

**4.5 AddCollateral** (borrower) → verify borrower position (collateral). Use enough collateral so that borrowing 50 YLDS (4.6) keeps resulting LTV below the pool's margin rate (e.g. 80%): at oracle price 1.5 and haircut 0.8, collateral value = amount × 1.5 × 0.8 = amount × 1.2; we need value ≥ 50M/0.8 = 62.5M, so amount ≥ 62.5M/1.2 ≈ 52.08M. Example: 55M units.

```bash
provenanced tx wasm execute $POOL '{"add_collateral":{}}' \
  --from borrower --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --amount 55000000$COLLATERAL_DENOM \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

```bash
provenanced query wasm contract-state smart $POOL '{"get_borrower_position":{"address":"'$BORROWER'"}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.6 Borrow** (borrower) → verify debt and health. Amount in base units (e.g. 50000000 = 50 YLDS). Borrow is only allowed if the resulting LTV stays below the pool's margin rate (e.g. 80%); the AddCollateral amount above is chosen so this borrow is valid at the doc's oracle prices. Borrowing 50 YLDS against 90 YLDS previously lent into the pool gives ~55% utilization so APR and interest accrual are visible.

```bash
provenanced tx wasm execute $POOL '{"borrow":{"amount":"50000000"}}' \
  --from borrower --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

```bash
provenanced query wasm contract-state smart $POOL '{"get_borrower_position":{"address":"'$BORROWER'"}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.7 Repay** (borrower) → verify debt decreased (e.g. repay 25 YLDS = 25000000):

```bash
provenanced tx wasm execute $POOL '{"repay":{}}' \
  --from borrower --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --amount 25000000$LENDING_DENOM \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

```bash
provenanced query wasm contract-state smart $POOL '{"get_borrower_position":{"address":"'$BORROWER'"}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.8 RemoveCollateral** → verify position:

```bash
provenanced tx wasm execute $POOL '{"remove_collateral":{"to_remove":{"'$COLLATERAL_DENOM'":"100000"}}}' \
  --from borrower --chain-id $CHAIN_ID \
  --keyring-backend test --testnet -b sync \
  --gas-prices 1nhash --gas-adjustment 2 -y
```

```bash
provenanced query wasm contract-state smart $POOL '{"get_borrower_position":{"address":"'$BORROWER'"}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.9 GetCollateralRequirements** (query only; useful for loan sizing):

```bash
provenanced query wasm contract-state smart $POOL '{"get_collateral_requirements":{"borrower":null,"new_loan_amount":"100000","collateral_assets":["'$COLLATERAL_DENOM'"]}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.10 Admin: UpdateSupportedCollateral** → verify via GetState or config:

```bash
provenanced tx wasm execute $POOL '{"update_supported_collateral":{"to_update":[{"id":"'$COLLATERAL_DENOM'","h":"0.95"}],"to_remove":[]}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
provenanced query wasm contract-state smart $POOL '{"get_state":{}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.10b Admin: UpdateRateParams** (switch fee model) → verify via GetState / GetReserve:

```bash
# Switch to flat spread mode (50 bps annual spread off borrower APR)
provenanced tx wasm execute $POOL '{"update_rate_params":{"rate_params":{"tr":"0.09","minr":"0.0325","maxr":"0.20","kink":"0.90","rf":"0.005","fm":"flat_borrow_spread","ff":"0.005","spy":31536000}}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y

# Switch back to reserve-factor mode (ff must be zero)
provenanced tx wasm execute $POOL '{"update_rate_params":{"rate_params":{"tr":"0.09","minr":"0.0325","maxr":"0.20","kink":"0.90","rf":"0.005","fm":"reserve_factor","ff":"0","spy":31536000}}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y

provenanced query wasm contract-state smart $POOL '{"get_state":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $POOL '{"get_reserve":{}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.11 Admin: WithdrawReserve** → verify reserve state:

```bash
provenanced tx wasm execute $POOL '{"withdraw_reserve":{"recipient":null}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
provenanced query wasm contract-state smart $POOL '{"get_reserve":{}}' --chain-id $CHAIN_ID --testnet -o json
```

**4.11b Optional: Commit-on-exit (commit_market_id, require_commit_on_exit)** — When using a Provenance exchange market for commit-on-withdraw: (1) set **commit_market_id** via UpdateContractConfig (required before setting require_commit for any lender); (2) admin sets **SetLenderRequireCommitOnExit** for an address so that they must pass commit_funds: true to withdraw and cannot use Transfer/TransferExact; (3) query **GetLenderStatus** to see require_commit_on_exit.

**Getting a market ID:** The exchange **create market request** is `MsgGovCreateMarketRequest`; it runs only as part of a **governance proposal**. You create a market via the CLI by submitting that proposal (e.g. `provenanced tx gov submit-proposal` with a proposal payload that includes the create-market message) — there is no dedicated `tx exchange create-market` command. Use an existing market ID from your network, or submit a gov proposal; if `market_id` is `0`, the next available ID is assigned. See [Exchange Messages](https://developer.provenance.io/docs/sdk/exchange/messages) and the [ValidateCreateMarket](https://developer.provenance.io/docs/sdk/exchange/queries#validatecreatemarket) query to validate the message before proposing.

**Submit a create-market gov proposal (minimal example):** Get the gov module address, then create a proposal JSON and submit it. Use the same `--testnet` / `--home` as elsewhere so the gov address matches your chain.

```bash
# Gov module address (use your chain's prefix: pb for localnet, tp for testnet)
export GOV_MODULE=$(provenanced q auth module-account gov -o json --chain-id $CHAIN_ID | jq -r '.account.value.address')
```

Save the following as `create_market_proposal.json` (set `$GOV_MODULE` in the `authority` field or replace it after export). This creates a market that only accepts commitments (suitable for pool `commit_market_id`); `market_id: 0` requests the next available ID.

```json
{
  "messages": [
    {
      "@type": "/provenance.exchange.v1.MsgGovCreateMarketRequest",
      "authority": "<GOV_MODULE_ADDRESS>",
      "market": {
        "market_id": 0,
        "market_details": {
          "name": "Pool commit market",
          "description": "Market for pool commit-on-withdraw",
          "website_url": "",
          "icon_uri": ""
        },
        "accepting_orders": false,
        "allow_user_settlement": false,
        "accepting_commitments": true,
        "commitment_settlement_bips": 0
      }
    }
  ],
  "metadata": "{\"title\":\"Create exchange market for pool commit\",\"summary\":\"Creates a single market for pool commit_funds.\"}",
  "deposit": "10000000nhash",
  "title": "Create exchange market for pool commit",
  "summary": "Creates a single market for pool commit_funds.",
  "expedited": false
}
```

Replace `<GOV_MODULE_ADDRESS>` with `$GOV_MODULE` in the file (e.g. `sed -i '' "s/<GOV_MODULE_ADDRESS>/$GOV_MODULE/g" create_market_proposal.json`), then submit:

```bash
provenanced tx gov submit-proposal create_market_proposal.json \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
```

**Vote the proposal through.** Submitting only creates the proposal; the market is created only after the voting period ends and the proposal passes. The tally is by **stake** (bonded tokens). On localnet the validator (e.g. node0) holds most of the stake, so have the validator vote — no need to delegate from admin.

```bash
# Vote with validator key (replace 1 with your proposal ID; localnet node0 key is in build/node0)
provenanced tx gov vote 1 yes --from node0 --home build/node0 --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash -y
# Check status / tally
provenanced q gov proposal 1 --chain-id $CHAIN_ID --testnet -o json
provenanced q gov tally 1 --chain-id $CHAIN_ID --testnet
```

After the voting period ends and the proposal passes, the create-market message is executed and the new market exists. To get the new market ID (for `commit_market_id`):

```bash
# List all markets (new one will have the next ID, e.g. 2)
provenanced q exchange all-markets --chain-id $CHAIN_ID --testnet -o json
# Or query a specific market by ID
provenanced q exchange market 2 --chain-id $CHAIN_ID --testnet -o json
```

Use the market’s `market_id` from the output as `commit_market_id` in UpdateContractConfig. Many chains have a default market (e.g. hash) as ID 1, so a newly created market is often ID 2.

Example (set commit_market_id and require_commit_on_exit):

```bash
# Set commit market (use your exchange market ID; at least one config field required)
provenanced tx wasm execute $POOL '{"update_contract_config":{"margin_rate":null,"liquidation_rate":null,"liquidation_bonus_rate":null,"price_oracle_address":null,"min_lend":null,"min_borrow":null,"max_borrower_collateral_types":null,"commit_market_id":1}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
# Require this lender to commit on exit (fails if commit_market_id not set)
provenanced tx wasm execute $POOL '{"set_lender_require_commit_on_exit":{"address":"'$LENDER'","require":true}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
provenanced query wasm contract-state smart $POOL '{"get_lender_status":{"address":"'$LENDER'"}}' --chain-id $CHAIN_ID --testnet -o json
# Withdraw with commit_funds (payload): use {"withdraw":{"amount":"...","commit_funds":true}} in the Send msg
# When require_commit_on_exit is not set, the user can still pass commit_funds: true to opt in to committing to the market.
```

**4.12 Admin: Withdraw** (withdraw a lender's supply on their behalf) → lender receives underlying; pool burns their repo token via the repo CW20. Does not check require_commit_on_exit. Optionally pass **`commit_funds: true`** to emit MsgCommitFundsRequest when `commit_market_id` is set. Use when closing a position or offboarding a lender. No `--amount` from admin; the pool sends from its balance to the lender. Use `amount: null` to withdraw the lender's full supply, or `amount: "5000000"` for a partial (e.g. 5 YLDS). After this step the lender's repo balance and pool liquidity decrease.

```bash
# Partial: withdraw 5 YLDS (5000000) on behalf of lender; they receive the underlying
provenanced tx wasm execute $POOL '{"withdraw":{"lender":"'$LENDER'","amount":"5000000"}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
# Same with commit to market (when commit_market_id is set)
# provenanced tx wasm execute $POOL '{"withdraw":{"lender":"'$LENDER'","amount":"5000000","commit_funds":true}}' \
#   --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
```

```bash
provenanced query wasm contract-state smart $POOL '{"get_state":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $POOL '{"get_reserve":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced query wasm contract-state smart $REPO '{"balance":{"address":"'$LENDER'"}}' --chain-id $CHAIN_ID --testnet -o json
```

To withdraw the lender's **full** supply instead, use `"amount":null`:

```bash
# Full: withdraw all of lender's supply on their behalf
# provenanced tx wasm execute $POOL '{"withdraw":{"lender":"'$LENDER'","amount":null}}' \
#   --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
```

**4.13 Admin: SetOperationalState** (frozen then active) → verify state:

```bash
provenanced tx wasm execute $POOL '{"set_operational_state":{"state":"frozen"}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
provenanced query wasm contract-state smart $POOL '{"get_state":{}}' --chain-id $CHAIN_ID --testnet -o json
provenanced tx wasm execute $POOL '{"set_operational_state":{"state":"active"}}' \
  --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync --gas-prices 1nhash --gas-adjustment 2 -y
```

**4.14 Optional: Liquidate** (admin repays debt and receives collateral; only when position is liquidatable). This step does **not** work in the default doc flow because:

1. **Liquidator must have YLDS** — The liquidator (admin) sends `--amount ...$LENDING_DENOM` to repay the borrower's debt; admin has no YLDS unless you mint or transfer some to them.
2. **Borrower must be liquidatable** — Liquidation is only allowed when the borrower's LTV ≥ pool `liquidation_rate` (e.g. 90%). After 4.5–4.7 (AddCollateral, Borrow, Repay), LTV is well below 90%, so the position is healthy.

To test liquidation you would: (a) ensure the borrower has debt and collateral; (b) **reprice** the collateral down in the price oracle so LTV ≥ 90% (e.g. `update_asset_prices` with a lower `usd` for `$COLLATERAL_DENOM`); (c) **fund admin** with YLDS (e.g. mint to admin or transfer from lender); (d) call **liquidate** with `amount` and `collateral_to_seize` matching the contract's rules (minimum repay to bring LTV healthy, seize value within 100–102% of repay). Template (adjust amounts and only run when the position is actually liquidatable and admin has the YLDS):

```bash
# Example: after repricing and funding admin with YLDS
# provenanced tx wasm execute $ORACLE_ADDRESS '{"update_asset_prices":{"prices":[{"asset":"'$COLLATERAL_DENOM'","usd":"0.5"}]}}' --from admin ...
# Then liquidate: repay amount is sent via --amount (one coin, lending denom); message has only borrower and collateral_to_seize (value with haircuts must be in [100%, liquidation_bonus_rate] of amount repaid).
# provenanced tx wasm execute $POOL '{"liquidate":{"borrower":"'$BORROWER'","collateral_to_seize":{"'$COLLATERAL_DENOM'":"<SEIZE_AMOUNT>"}}}' \
#   --from admin --chain-id $CHAIN_ID --keyring-backend test --testnet -b sync \
#   --amount <REPAY_AMOUNT>$LENDING_DENOM --gas-prices 1nhash --gas-adjustment 2 -y
# provenanced query wasm contract-state smart $POOL '{"get_borrower_position":{"address":"'$BORROWER'"}}' --chain-id $CHAIN_ID --testnet -o json
```

---

## 5. Suggested test order (high level)

1. **Prerequisites and deploy** (sections 1–3): keys, funding, markers, KYC names, oracle prices, then **either** Path A (repo → pool with `repo_token.existing` → UpdateConfig) **or** Path B (single pool instantiate with `repo_token.new`; set **$REPO** from `get_state` if needed). Grant lender.kyc.pb to pool and borrower (so borrower can receive YLDS on Borrow).
2. **Pool** (section 4): run steps 4.1–4.14 in order; each broadcast is followed by the listed queries. Repo token flows are exercised there; Path A also runs UpdateConfig at **3.1.3**.

Replace placeholders (`$POOL`, `$LENDER`, etc.) with real values. Use `--keyring-backend test --testnet` (and `--home build/node0` for localnet) as in the doc.

---

## References

- **Deployment order and message payloads:** [POOL_AND_REPO_TOKEN_DEPLOYMENT.md](POOL_AND_REPO_TOKEN_DEPLOYMENT.md) — Path A (existing repo token + UpdateConfig) vs Path B (`repo_token.new` in one tx), JSON payloads, and pitfalls. That doc is message-oriented; this doc adds copy-pastable CLI (sections 2–3 and 4).
- **pool_v2:** `contracts/pool_v2/` (CODEBASE_WALKTHROUGH.md, msg in `src/msg/`)
- **repo_token_cw20:** `contracts/repo_token_cw20/` (README). **`InstantiateMsg`** and token meta validation are shared via **`democratized_prime_lib::repo_token`** (re-exported from `src/msg.rs`).
- **price_oracle:** `contracts/price_oracle/` (for collateral prices and Borrow/Liquidate)
