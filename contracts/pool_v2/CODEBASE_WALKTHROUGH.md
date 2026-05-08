# Pool V2 Codebase Walkthrough

This document explains the **pool_v2** contract: an index-based lending pool with a kink interest rate model.

**Deployment with repo_token_cw20:** The pool uses a CW20 as its receipt token. Either **instantiate the pool with `repo_token.new`** (the pool creates the CW20 in a `SubMsg` and binds the address in `reply`, with minter and `pool_address` set to the pool—no separate `UpdateConfig`), or **use `repo_token.existing`** with a CW20 you deployed earlier (then run **UpdateConfig** on the CW20 to set minter and `pool_address` to the pool). See **[POOL_AND_REPO_TOKEN_DEPLOYMENT.md](../../docs/POOL_AND_REPO_TOKEN_DEPLOYMENT.md)** for both paths and CLI-oriented playbooks.

**Shared instantiate payload:** For **`repo_token.new`**, the pool builds **`democratized_prime_lib::repo_token::InstantiateMsg`** and serializes it into the SubMsg. Name/symbol/decimals are validated with **`validate_repo_token_meta`** from the same module so they stay aligned with **`repo_token_cw20`**’s `instantiate`.

---

## 1. Models (`src/model/`)

Models define the **data shapes** used by the contract: what gets stored, what gets sent in messages, and what gets returned from queries.

### 1.1 `RateParamsV1` (`rate_params.rs`)

**Purpose:** Config for the **kink interest rate curve**.

| Field | Meaning |
|-------|--------|
| `target_rate` | Borrower APR at the kink (e.g. 9% when utilization = kink). |
| `min_rate` | Borrower APR when utilization = 0 (e.g. 3.25%). **The borrow index accrues at this rate even when there are zero borrows** (lender rate is 0 at 0% utilization, so the liquidity index does not grow until there is utilization). Spreadsheets that assume “borrow index = 1 until the first borrow” will show different index (and scaled) values. |
| `max_rate` | Borrower APR when utilization = 100% (e.g. 20%). |
| `kink_utilization` | Utilization where the curve kinks (e.g. 0.9 = 90%). Below this, rate rises slowly; above, rate rises steeply. |
| `reserve_factor` | Share of borrower interest kept by the protocol (e.g. 0.005 = 0.5%). Lender rate = borrower_rate × utilization × (1 − reserve_factor). |
| `seconds_per_year` | Used for index growth (e.g. 31_536_000). |

Set at **instantiation** and used whenever we compute borrower/lender rates from utilization and when we grow indexes.

---

### 1.2 `ReserveStateV1` (`reserve.rs`)

**Purpose:** The **reserve state**: indexes, aggregate scaled lend/borrow, and protocol reserve accrual. This is the core of the index-based design.

| Field | Meaning |
|-------|--------|
| `liquidity_index` | Grows over time as lender interest accrues. **User balance (underlying) = scaled_balance × liquidity_index.** |
| `borrow_index` | Grows over time as borrower interest accrues. **User debt (underlying) = scaled_borrow × borrow_index.** |
| `last_updated_at` | Last time these indexes were updated (for accrual). |
| `total_scaled_liquidity` | Sum of all lenders’ scaled balances (never decreases except on withdraw). |
| `total_scaled_borrow` | Sum of all borrowers’ scaled debt (decreases on repay, liquidation, and **bad-debt** write-off). |
| `accrued_reserve` | Protocol share of interest (reserve factor), in lending base units. Updated whenever indexes accrue: `accrued_reserve += (borrower_interest_delta − lender_interest_delta)` for that period. |
| `deficit_underlying` | **Shortfall** in lending base units after a **bad-debt** liquidation: the borrower has no collateral left, but a slice of debt could not be repaid from what was seized. It is **not** scaled borrow. It does **not** earn borrower interest. Older deployments deserialize it as **0** (`#[serde(default)]`). |

**Two “cash” ideas (underlying units):**

- **Solvency (invariant tests):** **implied cash** = `total_liquidity + accrued_reserve − total_borrow − deficit_underlying`. Must stay ≥ 0.
- **Borrow / withdraw cap:** **free cash** = `total_liquidity − total_borrow − deficit_underlying`, from `reserve_totals_and_cash_u128` and `ReserveStateV1::cash()` (subtractions **saturate at zero** so the value is never negative). **`accrued_reserve` is not in this expression**—the cap compares lender-side aggregate supply to borrows and to booked deficit. Executes still require **`amount ≤ cash`** (and their other amount rules).

**Utilization** (for the kink curve and `GetReserve`) is **`total_borrow / total_liquidity`** (0 if there is no liquidity). **`deficit_underlying` is not part of that fraction**; it only reduces **free cash** above. After bad debt, **`total_borrow` drops** (debt is written off) **and** **`deficit_underlying` rises** (in **deferred** loss mode), so **read utilization together with `deficit_underlying`** (and implied cash) when judging pool stress.

The contract owner withdraws protocol fees with **WithdrawReserve**. That is **disabled while `deficit_underlying` is positive**. Use **EliminateDeficit** first.

**Derived (methods):**

- `total_liquidity()` = total_scaled_liquidity × liquidity_index (total supplied, in underlying).
- `total_borrow()` = total_scaled_borrow × borrow_index (total borrowed, in underlying).
- `cash()` = total_liquidity − total_borrow − deficit_underlying (underlying `Decimal256`; saturating sub).
- `utilization()` = total_borrow / total_liquidity (0 if no liquidity). Drives the interest rate.

We **store** one `ReserveStateV1` (singleton). On **queries**, we use an **effective** reserve: same struct but with indexes **accrued to current block time** (read-only, not saved), so responses show current balances, current rates, and current accrued_reserve.

---

### 1.3 `ContractStateV1` (`contract_state.rs`)

**Purpose:** **Contract configuration** (metadata, **repo token** CW20 address, denom, rate params, attribute gates, oracle, LTV/liquidation params, mins, supported collateral, optional **commit market**, operational state). The **contract owner** is stored by **cw-ownable**, not on this struct.

| Field | Meaning |
|-------|--------|
| *(owner)* | **cw-ownable** holds the address authorized for owner-only executes (not a field on `ContractStateV1`). |
| `contract_name`, `description` | Metadata. |
| `lending_denom` | The asset lent/borrowed (e.g. YLDS). |
| `rate_params` | Kink model parameters (see above). |
| `lender_required_attrs`, `borrower_required_attrs` | List of Provenance attribute names: sender must have **all** of these to lend (or receive Transfer) / to borrow (or add/remove collateral). Empty list = no attribute required. Updatable by the contract owner via **SetLenderRequiredAttrs** / **SetBorrowerRequiredAttrs**. |
| `price_oracle_address` | Contract used for collateral prices (for LTV/health). |
| `max_borrower_collateral_types` | Max number of collateral asset types per borrower. |
| `margin_rate`, `liquidation_rate` | LTV thresholds: at or below margin_rate = healthy (can borrow); at or above liquidation_rate = liquidatable. |
| `liquidation_bonus_rate` | Max collateral value seized = repay value × this (e.g. 1.02 = 2%); must be > 1. |
| `min_lend` | Minimum lending amount. |
| `min_borrow` | Minimum borrow amount (independent of min_lend). |
| `supported_collateral_assets` | Supported collateral assets (asset_id and optional haircut). The contract owner can update via **UpdateSupportedCollateral**. |
| `repo_token_cw20_address` | CW20 contract address for the repo (receipt) token. Set at instantiate with **`RepoTokenConfig::Existing`**; with **`RepoTokenConfig::New`**, **`None` until `reply`** after the repo-token `SubMsg` succeeds, then set for the rest of the tx. Lend mints and Withdraw burns this CW20. Lender balance is obtained by querying **repo_token_cw20**'s CW20 **Balance** (and **TokenInfo** for total lending); when `pool_address` is set on the CW20, it returns underlying. |
| `commit_market_id` | Optional Provenance exchange market ID. When set, user Withdraw/WithdrawExact with **commit_funds: true** causes the pool to emit **MsgCommitFundsRequest** (funds are re-committed to the market on exit). Set at **instantiate** or via **UpdateContractConfig** (cannot be cleared once set). Required before the contract owner can set per-lender "require commit on exit" (SetLenderRequireCommitOnExit). |
| `bad_debt_loss_allocation` | **BadDebtLossAllocation** (default **deferred_to_deficit**): on bad-debt liquidation, either book **`deficit_underlying`** (then **EliminateDeficit** / **SocializeDeficit**) or apply an immediate pro-rata **`liquidity_index`** haircut (**immediate_liquidity_index_haircut**). Set at **instantiate** or **UpdateContractConfig** (allocation may change only when **`deficit_underlying`** is zero). |
| `operational_state` | **Active** / **Frozen** / **Paused**. **Frozen:** blocks new **Lend** and **Borrow** only. **Paused:** strictest—blocks user value movement (**Liquidate**, **Repay**, **Receive**, **Withdraw**, deficit clears, collateral moves, **WithdrawReserve**); only a small set of owner config executes (see **`contract.rs`**). Set via **SetOperationalState**. |

Stored as a **singleton** at instantiation. Further owner-controlled updates match **ExecuteMsg** (Section 3).

---

### 1.4 `Denom` (`denom.rs`)

**Purpose:** Identifies the **lending asset** (name + precision).

- `name`: e.g. `"uylds.fcc"`.
- `precision`: decimal places for the coin.
- Helpers: `to_cw_coin(amount)`, `to_prov_coin(amount)`, `validate()`.

Used in contract state and whenever we build coins to send or validate.

---

### 1.5 Collateral (`collateral.rs`)

**Purpose:** Defines **which assets can be used as collateral** and **per-borrower collateral balances**.

- **`CollateralAssetV1`**: One supported collateral type: `asset_id` (e.g. marker denom) and optional `haircut` (discount for valuation, 0–100%).
- **Supported collateral**: Stored on **`ContractStateV1`** as `supported_collateral_assets: Vec<CollateralAssetV1>` (asset_id and optional haircut). The contract owner can update via **UpdateSupportedCollateral**. The price oracle is also on `ContractStateV1` and used for all price queries.
- **`BorrowerCollateralV1`**: Per-borrower: `amounts: HashMap<asset_id, u128>`. Stored **per borrower address**.

Used for borrow health checks and liquidations (LTV vs margin_rate / liquidation_rate). Health is implemented: get_borrower_health, validate_borrower_is_healthy; Borrow and RemoveCollateral enforce healthy LTV; Liquidate requires LTV ≥ liquidation_rate. What happens when a borrower's collateral no longer covers their debt (LTV too high) is described in **Section 5 (When collateral doesn't cover debt)**.

---

### 1.6 Query response types (`state.rs`, `query.rs`)

**Purpose:** **Aggregate view** returned by the **GetState** query.

- **StateResponseV1** (`state.rs`): Returned by **GetState**. `contract: ContractStateV1`, `reserve: ReserveStateResponseV1`, **supported_collateral** (allowed assets + haircuts), and **total_collateral_held** (per-asset totals held in the pool, sum across all borrowers). So one call returns full pool config plus lending-side (reserve) and collateral-side (allowed + held) state.
- **ReserveStateResponseV1** (`query.rs`): Reserve in API responses. Same logical fields as `ReserveStateV1` (indexes, scaled totals, accrued_reserve, **deficit_underlying**) as strings, plus **total_liquidity** and **total_borrow** (scaled × index) so clients don't have to compute them.
- **ReserveResponseV1**: GetReserve returns `reserve: ReserveStateResponseV1` plus current_borrower_rate, current_lender_rate, utilization.

So “state” = config + reserve + supported collateral + total collateral held; the reserve in the response is effective (indexes accrued to block time) and includes total_liquidity / total_borrow.

---

### 1.7 `error.rs`

**Purpose:** Re-exports **`ContractError`** and **`QueryError`** (and helpers like `illegal_argument`, `not_authorized`, `illegal_state`) from the shared lib. No custom error types in pool_v2; we use the lib’s execution/query errors.

---

## 2. Storage (`src/storage/`)

**What we persist:**

| Module | What | Key pattern |
|--------|------|-------------|
| `contract_state` | ContractStateV1 | Singleton `Item` |
| `reserve` | ReserveStateV1 | Singleton `Item` |
| `collateral` | BorrowerCollateralV1 per borrower + total by asset | `Map<borrower_addr, BorrowerCollateralV1>` + `Map<asset_id, u128>` |
| `scaled_borrow` | Per-borrower scaled debt (u128) | `Map<borrower_addr, u128>` |

So we have:

- **Singletons:** contract config (includes supported_collateral_assets), reserve state.
- **Maps:** scaled borrow by borrower, borrower collateral by borrower. **Lender balances are not stored in the pool;** the **repo token** is a **CW20** at `repo_token_cw20_address` (Lend mints it, Withdraw burns it). Lender balance is read via the **repo_token_cw20** contract's CW20 **Balance** query.

No “raw” underlying balances are stored for lend/borrow; only scaled amounts and indexes. Underlying = scaled × index (with index from reserve, or effective reserve in queries).

---

## 3. Messages (`src/msg/`)

**InstantiateMsg:** Contract name, description, **`repo_token`** (**`existing`** with `repo_token_cw20_contract_address`, or **`new`** with code id + name/symbol/decimals to instantiate `repo_token_cw20` in the same tx—the SubMsg uses **`democratized_prime_lib::repo_token::InstantiateMsg`**), lending denom, **rate_params** (kink model), lender/borrower attributes, price oracle, collateral limits, LTV params, **min_lend**, **min_borrow**, supported collateral assets.

**ExecuteMsg:**

- **Lend** – User sends coins; we mint repo token to them and add to total_scaled_liquidity (after accruing indexes).
- **Receive** – Called by the repo CW20 when a user Sends tokens to the pool. Payload is **Withdraw** (underlying amount; optional **commit_funds: true**; we burn >= required scaled repo token, send underlying, refund excess; when this lender has **require_commit_on_exit** they must pass commit_funds: true; when commit_funds is true, **commit_market_id** must be set or the call fails, then we emit MsgCommitFundsRequest), **WithdrawExact** (same commit_funds/commit behavior), **Transfer**, or **TransferExact**. **Transfer/TransferExact are not allowed** when the sender has require_commit_on_exit (they must withdraw with commit_funds first). So *user* withdraw/transfer is via CW20 Send to the pool, not a direct pool execute. **Allowed when Frozen** (only Lend and Borrow are blocked in that state); blocked when Paused.
- **Borrow** – User borrows amount; we add to their scaled borrow and total, send coins (after accruing indexes). Requires healthy LTV (get_borrower_health, validate_borrower_is_healthy).
- **Repay** – User sends coins; we reduce their scaled borrow and total (after accruing).
- **AddCollateral** / **RemoveCollateral** – Add/remove collateral (from funds or specified amounts). Update BorrowerCollateralV1; RemoveCollateral validates LTV stays healthy.
- **Liquidate** – **Contract owner only**. Target borrower must be liquidatable (LTV ≥ liquidation_rate). The liquidator sends one lending coin; repay is **min(sent, debt)** and must meet the computed minimum. They also choose **collateral_to_seize**; its **market** value must sit between 100% and the liquidation bonus multiple of the repay value. Seized collateral goes to the liquidator; debt and collateral are reduced on chain. **Bad debt path:** if the seizure would leave **no collateral** but **debt still on the books**, the contract zeroes that borrower’s debt and trims **total_scaled_borrow** correctly. Depending on **`bad_debt_loss_allocation`**, it either books **`deficit_underlying`** (**deferred**) or applies an immediate **`liquidity_index`** haircut (**immediate**). Attributes **`bad_debt_underlying`**, **`deficit_underlying`**, and **`bad_debt_loss_allocation`** describe the path. **Blocked when Paused.**
- **Withdraw** – **Contract owner only.** Withdraw a lender’s supply on their behalf: specify **lender**, optional **amount** (max underlying; if None, withdraws full supply), and optional **commit_funds: true**. Does **not** check require_commit_on_exit; when **commit_funds** is true, **commit_market_id** must be set or the call fails, and the pool emits MsgCommitFundsRequest for the withdrawn amount. Pool burns the lender’s repo token via the repo CW20’s **BurnFrom** (minter burns from `owner`’s balance; no tokens sent to the pool). Sends underlying to the lender. Use when closing a pool or for operational exits. **Blocked when Paused.**
- **UpdateSupportedCollateral** – Contract owner: add/update/remove supported collateral assets (cannot remove an asset in use).
- **WithdrawReserve** – Contract owner only; no funds. Sends the full **`accrued_reserve`** in the lending asset (optional recipient; else contract owner). Indexes are brought current first. **Not allowed if there is a deficit**—clear **`deficit_underlying`** with **EliminateDeficit** before pulling protocol fees.
- **EliminateDeficit** – Reduces **`deficit_underlying`**. Pick **one** source per call: **`accrued_reserve`** (**contract owner only**; uses the fee bucket on the reserve; do not attach coins) or **`bank`** (**any sender**; attach one lending coin). Each mode takes at most the requested cap, the remaining deficit, and what that source actually provides. **Bank** refunds anything sent above what was applied (to the sender). **Paused:** blocked. **Frozen:** still allowed.
- **SocializeDeficit** – Contract owner only; no funds. Haircut on **`min(max_amount, deficit_underlying)`** (same as immediate bad-debt path); fails if applied loss is not strictly below **`total_liquidity`**. **Paused:** blocked. **Frozen:** still allowed.
- **SetOperationalState** – Contract owner only; no funds. Switches **Active**, **Frozen**, or **Paused**. **Paused** is the strictest: only a handful of owner config messages run; all user value movement (including **Liquidate**, **EliminateDeficit**, and **SocializeDeficit**) is off until you return to Active or Frozen.
- **SetLenderRequiredAttrs** – Contract owner only; no funds. Sets the list of required lender attributes (Lend and Transfer recipient must have **all**). Empty list = no check.
- **SetBorrowerRequiredAttrs** – Contract owner only; no funds. Sets the list of required borrower attributes (Borrow, AddCollateral, RemoveCollateral require **all**). Empty list = no check.
- **UpdateContractConfig** – Contract owner only; no funds. Optional fields: margin_rate, liquidation_rate, liquidation_bonus_rate, price_oracle_address, min_lend, min_borrow, max_borrower_collateral_types, **commit_market_id** (integer = set Provenance market id; omit field or JSON `null` = no change to this field; there is no “clear” via this message), **bad_debt_loss_allocation** (may change only when **`deficit_underlying`** is 0; repeating the current value is allowed). Only provided (non-null) fields are updated. After apply, invariants are checked: margin_rate &lt; liquidation_rate, liquidation_bonus_rate &gt; 1, bonus × margin_rate &lt; 1. **Liquidation rate may only be increased** (never decreased), so the contract owner cannot lower the LTV bar and make previously safe positions liquidatable.
- **SetLenderRequireCommitOnExit** – Contract owner only; no funds. Per-address flag: **require: Some(true)** = this lender must pass **commit_funds: true** on Withdraw/WithdrawExact and **cannot** use Transfer/TransferExact (they must withdraw with commit to return funds to the committed market first). **require: Some(false)** or **None** = clear/remove. **commit_market_id must be set** before setting require: true (otherwise the call fails).
- **UpdateRateParams** – Contract owner only; no funds. Full replacement of rate_params (kink model); validated same as at instantiate. The contract **accrues reserve indexes to the current block** with the old params first, then applies the new params, so the new curve applies only **from this block onward**. Does not change who is liquidatable (that is margin_rate/liquidation_rate in contract config).

**QueryMsg:**

- **GetState** – Contract + **effective** reserve (indexes to current block) + **supported_collateral** (allowed assets and haircuts) + **total_collateral_held** (amount of each collateral asset held in the pool). Contract includes the repo CW20 address (`repo_token_cw20_address` in Rust; JSON key **`atca`** on the embedded contract object).
- **GetReserve** – **Effective** reserve + current_borrower_rate, current_lender_rate, utilization.
- **GetBorrowerPosition(address)** – Borrower position: debt (scaled + underlying + display), `collateral` (per-asset amounts), `collateral_value_usd`, `loan_to_value`, and `health` (healthy / unhealthy / liquidatable / no_collateral / unknown). Uses oracle for prices.
- **Lender balance:** The pool does not expose a BalanceOf query. Query the **repo_token_cw20** contract (address in GetState) with the standard CW20 **Balance** (and **TokenInfo** for total supply); when the CW20's pool_address is set, it returns underlying amounts.
- **Displaying amounts:** Use the CW20 Balance/TokenInfo for lent supply; use `underlying_debt_display` from GetBorrowerPosition for debt. Pair with `lending_denom.name` for the unit label.
- **GetCollateralRequirements** – Optional borrower, new loan amount, and collateral asset ids. Returns required total collateral value (USD), additional value needed (when borrower set), and per-asset minimum amounts. See **Section 6.2** for the full flow.
- **GetLenderStatus(address)** – Returns **require_commit_on_exit**: whether this address must pass commit_funds: true to withdraw and is blocked from Transfer/TransferExact.

---

## 4. Rate logic and scaling (`src/utils/rates.rs`)

**Kink model (borrower rate):**

- If utilization ≤ kink:
  `rate = min_rate + (utilization / kink) × (target_rate − min_rate)`
- If utilization > kink:
  `rate = target_rate + (utilization − kink) / (1 − kink) × (max_rate − target_rate)`

**Lender rate:**
`lender_rate = borrower_rate × utilization × (1 − reserve_factor)`

**Index growth (per time step):**
`new_index = old_index × (1 + rate × elapsed_seconds / seconds_per_year)`

**Functions:**

- **`borrower_rate_from_utilization(params, utilization)`** – Returns the borrower APR (Decimal256) for a given utilization using the kink model: below kink a linear slope from min_rate to target_rate, above kink a steeper slope from target_rate to max_rate. Used when accruing interest to compute the rate that applies to the current utilization.

- **`lender_rate_from_utilization(params, utilization, borrower_rate)`** – Returns the lender APR as `borrower_rate × utilization × (1 − reserve_factor)`. Lenders earn a share of borrower interest, reduced by utilization and the reserve factor (protocol share). Used together with the borrower rate when growing the liquidity index.

- **`index_growth_factor(rate, elapsed_seconds, seconds_per_year)`** – Returns the multiplicative factor for one index step: `1 + rate × (elapsed_seconds / seconds_per_year)`. New index = old index × this factor. Returns 1 if elapsed is 0. Used by `compute_effective_reserve` to grow liquidity and borrow indexes.
- **`compute_effective_reserve(store, as_of_time, params)`** – **Read-only**: loads the stored reserve, accrues liquidity and borrow indexes from `last_updated_at` to `as_of_time` using **current** utilization and the kink model, and adds (borrower_interest_delta − lender_interest_delta) to **accrued_reserve** for that period. Returns the updated reserve **without** saving. Used by all queries so callers see current indexes, balances, and rates; also used by `update_reserve_indexes`. At 0% utilization, borrower rate = min_rate and lender rate = 0, so the borrow index grows and the liquidity index does not.
- **`update_reserve_indexes(store, env, params)`** – Calls `compute_effective_reserve(store, env.block.time, params)` then **saves** the result (including updated indexes and accrued_reserve). Used at the start of Lend, Withdraw, WithdrawExact, Borrow, Repay, RemoveCollateral, and Liquidate so state is accrued before mutating.
- **Scaling helpers** (Decimal256 with 18-decimal atomics ↔ u128):
  - **`underlying_to_scaled_liquidity(underlying, liquidity_index)`** – Floor. Use when *recording* new lend supply and when *reducing* liquidity (e.g. withdraw, transfer): floor on mint keeps booked lender claims from exceeding received coins; floor on withdraw converts requested underlying to scaled units to deduct so we never deduct more scaled than the request entitles. For **Withdraw**, the pool must send **scaled_to_underlying(scaled)** (the value of what we burn), not the requested amount—otherwise floor(amount/index)×index &lt; amount would over-credit the user and leak from the pool.
  - **`underlying_to_scaled_borrow(underlying, borrow_index)`** – Floor. Use when *reducing* debt (Repay): convert repay amount to scaled debt to subtract; floor avoids over-reducing debt.
  - **`underlying_to_scaled_borrow_ceil(underlying, borrow_index)`** – Ceil. Use when *adding* debt (Borrow execute): we lend `underlying` and record scaled debt; ceil ensures recorded debt is never less than what we lent.
  - **`scaled_to_underlying_liquidity(scaled, liquidity_index)`** – Floor/truncate. Use for balance queries and withdraw/transfer limits: “how much can the user withdraw?” Truncation keeps withdrawable slightly under the true value; dust stays in the pool.
  - **`scaled_to_underlying_borrow(scaled, borrow_index)`** – Floor/truncate. Use for debt queries, repay caps, and liquidation: “how much does the user owe?” Truncation avoids over-stating debt; rounding is in the protocol’s favor.

So: **execute** updates stored indexes; **queries** use effective indexes so the user always sees current balances and current rates.

---

## 5. Execute flow

What each message does is summarized in **Section 3 (ExecuteMsg)**. Below is the detailed flow by module, with formulas and implementation notes.

### 5.2 Detailed flow (by module, with formulas)

Lend (`lend.rs`)
The user sends lending-denom coins and must have the lender attribute. The contract accepts exactly one coin, of the lending denom, at or above the minimum lent supply. It accrues interest (updates reserve indexes and accrued_reserve), then converts the supplied amount into “scaled” units using the current liquidity index (**floor** so booked supply never exceeds coins received; sub-unit dust stays in the pool). It adds that scaled amount to total scaled liquidity and mints the same scaled amount of repo token to the user. So: more lent supply → more repo token; the user’s share of the pool is their repo token balance times the liquidity index.

**Withdraw** (`withdraw.rs`)  
The user names an amount in **underlying**. If **require_commit_on_exit** applies to them, they must pass **commit_funds: true** or the call fails. After accruing indexes, the pool checks the request against **available cash** (`reserve_totals_and_cash_u128`: lending supplied, minus borrows, minus any **deficit**). It converts to scaled repo, burns what the user sent, and pays out the **value of what was burned** (not the raw request string), so rounding cannot drain the pool. Optional **commit_funds** + **commit_market_id** re-commit withdrawn funds on Provenance (**MsgCommitFundsRequest**). Excess repo is refunded. Use **WithdrawExact** to withdraw “everything.”

**WithdrawExact** (`withdraw.rs`)
Same commit rules as **Withdraw**, but the size comes from the **repo token in the message**, not from a separate amount field. After accruing, the pool checks that the implied underlying is within **available cash** (same formula as Withdraw), then burns and pays. Typical “withdraw all” flow: query balance, send that much repo in one message.

**Borrow** (`borrow.rs`)
After accruing indexes, the pool checks **available cash** the same way as for a withdraw (supplied liquidity, minus borrows, minus deficit). It then prices collateral and rejects the loan if post-borrow LTV would exceed the **margin** (position must stay healthy). On success it records scaled debt (rounded **up** so the pool never understates what it lent) and sends the lending coins.

**Repay** (`repay.rs`)
The user sends lending-denom coins. The contract accrues interest and takes a single repayment coin. It repays only up to the borrower’s current debt (if they send more, the rest is effectively excess). It converts that repayment into scaled units and subtracts it from the borrower’s scaled debt and from total scaled borrow. No separate “send” is needed—the repayment is the coins in the message.

**AddCollateral** (`add_collateral.rs`)
The user sends one or more coins that must be supported collateral types. The contract checks the borrower attribute and that the number of distinct collateral types (existing plus new) does not exceed the limit. It adds the received amounts to the borrower’s collateral balances and to the running totals per asset. The coins remain in the contract (they are the collateral).

**RemoveCollateral** (`remove_collateral.rs`)
The user specifies which assets and how much to remove (e.g. “500 of denom A, 100 of denom B”). The contract checks the borrower attribute and that the requested amounts do not exceed what the borrower has. It accrues interest (to value debt correctly), then imagines the position *after* removing that collateral. It recomputes LTV for that hypothetical position; if LTV would be above the margin rate, the removal is rejected so the position stays healthy. If allowed, it subtracts the amounts from the borrower’s collateral and from the totals, then sends those coins back to the user.

**Transfer** (`transfer.rs`)  
The user specifies a recipient and an amount in *underlying* (e.g. “1000 units’ worth of repo token”). If the **sender** has **require_commit_on_exit** set, Transfer (and TransferExact) are **not allowed**—they must withdraw with commit_funds first. The recipient must have the lender attribute. The contract does *not* accrue reserve indexes (transfer doesn’t change pool totals). It converts the underlying amount to scaled repo token using the current liquidity index, and requires the sender to have sent at least that many repo tokens in the message. It sends that scaled amount to the recipient and refunds any excess repo token to the sender. Total liquidity in the pool is unchanged.

**TransferExact** (`transfer.rs`)
The user sends one coin (repo token) and specifies only the recipient; the amount is taken from the funds. The contract validates recipient and lender attribute, then sends the full sent amount to the recipient. Use for “transfer all”: query scaled balance, then send TransferExact { recipient } with that much repo token in funds.

**When collateral doesn't cover debt**
A borrower's position has **LTV = debt_value / collateral_value** (collateral valued with haircuts). If debt grows (interest accrues) or collateral value falls (oracle prices drop), LTV rises. The contract does **not** automatically close or adjust the position. Instead:

- **LTV ≤ margin_rate:** Healthy. The borrower can borrow more and remove collateral (subject to checks).
- **LTV > margin_rate but < liquidation_rate:** Unhealthy; the borrower cannot borrow more or remove collateral until they repay or add collateral to bring LTV to at or below margin_rate.
- **LTV ≥ liquidation_rate:** **Liquidatable** (still requires a **Liquidate** tx). A run pays down debt and pulls collateral within the value band. If seizure uses up all collateral but **debt remains**, the **bad-debt** path clears leftover scaled debt; supplier impact follows **`bad_debt_loss_allocation`**: **`deficit_underlying`** (**deferred**) or an immediate **`liquidity_index`** haircut (**immediate**). A **deferred** deficit can be reduced later with **EliminateDeficit** (**`bank`** from any payer or **`accrued_reserve`** by the contract owner) or **SocializeDeficit** (pro-rata haircut).

**Liquidate** (`liquidate.rs`)
Contract owner only. The contract accrues interest and loads the borrower's debt and collateral. The borrower must be *liquidatable*: **LTV ≥ liquidation_rate**.

The contract computes the **minimum repay**: the repayment (in lending units) that would bring LTV back to exactly the margin rate, assuming we seize collateral whose value is between 100% and the liquidation-bonus rate (e.g. 102%) of that repay. This minimum is capped by the borrower's full debt and by total collateral value (so partial liquidation is possible). The contract owner sends lending denom in **funds** (one coin); actual repay = min(sent, debt), and sent must be ≥ minimum. Excess is refunded. There is no `amount` field in the message (same pattern as Repay).

The contract owner also specifies *which* collateral to seize (denom and amount per asset). The **market value** of that collateral (price × amount; no haircut) must lie in the band: **≥ repay value** and **≤ repay value × liquidation_bonus**. So the liquidator receives collateral worth at least what they repaid, but not more than the bonus cap (profit is capped as intended).

The contract simulates collateral **after** the seizure. **Normal case:** some collateral remains; usual partial repay and debt update. **Bad-debt case:** no collateral would remain but debt would not be fully cleared—the borrower’s scaled debt is cleared, **total_scaled_borrow** drops by repay plus write-off, and reserve updates follow **`bad_debt_loss_allocation`** (deferred vs immediate; see Section 3). Response attributes include **`bad_debt_underlying`**, **`deficit_underlying`**, and **`bad_debt_loss_allocation`**. Collateral totals and borrower map are saved, seized coins go to the liquidator, and rate metadata is attached.

**Owner withdraw (on behalf of lender)** (`withdraw.rs`, **ExecuteMsg::Withdraw**)  
Contract owner only. Specify **lender** address, optional **amount** (max underlying; if None, full supply), and optional **commit_funds: true**. Does **not** check the lender’s require_commit_on_exit flag. When **commit_funds** is true, **commit_market_id** must be set on the contract or the call fails; when set, the pool emits **MsgCommitFundsRequest** for the withdrawn amount (same as user withdraw with commit_funds). The contract accrues interest, looks up the lender’s scaled balance (via CW20 query), computes withdrawable underlying, caps by **amount** if provided, then calls the repo CW20’s **BurnFrom { owner: lender, amount: scaled_to_burn }** so the minter (pool) burns from the lender’s balance—no repo token is sent to the pool. Underlying is sent to the lender. **Blocked when Paused** (no funds leave the pool during an emergency). **Burn vs BurnFrom:** user withdraw (Receive) uses **Burn** (pool burns its own balance after the user Sent repo token to the pool); owner withdraw on behalf of the lender uses **BurnFrom** (pool burns the lender’s balance directly).

**UpdateSupportedCollateral** (`update_supported_collateral.rs`)
Contract owner only; no funds. The contract owner can add or update supported collateral assets (asset id and haircut) and can list asset ids to remove. The contract will not remove an asset that any borrower still has a balance of. It merges the updates and removals into contract state's `supported_collateral_assets` and saves contract state.

**SetLenderRequiredAttrs** (`set_lender_required_attrs.rs`)
Contract owner only; no funds. Sets the list of required lender attributes. Sender must have **all** of these to Lend or to receive a Transfer. Empty list = no attribute check.

**SetBorrowerRequiredAttrs** (`set_borrower_required_attrs.rs`)
Contract owner only; no funds. Sets the list of required borrower attributes. Sender must have **all** of these to Borrow, AddCollateral, or RemoveCollateral. Empty list = no attribute check.

**UpdateContractConfig** (`update_contract_config.rs`)
Contract owner only; no funds. Takes optional fields only; at least one must be provided. Applies each provided field to contract state, then validates: margin_rate &lt; liquidation_rate, liquidation_bonus_rate &gt; 1, bonus × margin_rate &lt; 1, min_lend/min_borrow non-zero if updated, price_oracle non-empty valid address if updated. **liquidation_rate may only be increased** (new value ≥ current); decreasing is rejected to avoid making previously safe positions liquidatable. **bad_debt_loss_allocation** may change only when **`deficit_underlying`** is zero (re-stating the current enum is still allowed). Saves contract state.

**UpdateRateParams** (`update_rate_params.rs`)
Contract owner only; no funds. Takes full `rate_params: RateParamsV1`. Validates, then **accrues reserve indexes to current block time** using the current (old) rate params (`update_reserve_indexes`), then sets contract.rate_params to the new value and saves. So the new curve applies only from this block onward; past accrual up to “now” used the old curve. Does not alter scaled amounts or who is liquidatable (that depends on margin_rate/liquidation_rate in contract config).

**WithdrawReserve** (`withdraw_reserve.rs`)
Contract owner only; no coins attached. Brings indexes current, then fails if **`deficit_underlying`** is still positive. Otherwise pays out the whole **`accrued_reserve`** (optional recipient; default contract owner) and zeros that bucket. No-op if there is nothing accrued.

**EliminateDeficit** (`eliminate_deficit.rs`)
Indexes are brought current, then **either** **`accrued_reserve`** (contract owner only; no coins on the message) **or** **`bank`** (any sender; one lending coin in funds)—not both. The applied amount is capped by the per-call limit, the live deficit, and what the source actually has. **Bank** refunds any overpayment to **`info.sender`**. Response carries how much moved and what deficit is left. **Paused:** not accepted. **Frozen:** accepted.

**SocializeDeficit** (`socialize_deficit.rs`)
Contract owner only; no funds. **`apply_pro_rata_liquidity_index_haircut`** on **`min(max_amount, deficit_underlying)`**; errors if loss not strictly below **`total_liquidity`**. **Paused:** blocked. **Frozen:** allowed.

---

## 6. Query flow

What each query returns is summarized in **Section 3 (QueryMsg)**. All reserve-dependent queries use **compute_effective_reserve(storage, env.block.time, rate_params)** so indexes are current; no state is written. Below is the detailed flow by module, with formulas.

### 6.2 Detailed flow (by module, with formulas)

**GetState** (`query/state.rs`)
Returns the full public state: contract config, the **effective** reserve (indexes and accrued_reserve accrued to current block time), **supported_collateral** (allowed assets and haircuts), and **total_collateral_held** (total amount of each collateral asset currently held in the pool, sum across all borrowers). So one query gives lending-side (reserve) and collateral-side (config + held) state. Nothing is written to storage.

**GetReserve** (`query/reserve.rs`)
Same effective reserve as above, plus the **current** borrower and lender APRs and utilization. Utilization is total borrow divided by total liquidity. The borrower rate comes from the kink curve at that utilization; the lender rate is borrower rate × utilization × (1 − reserve factor). So the UI can show “current borrow APR” and “current lend APR” as of the block.

**Lender balance:**
The pool does not have a BalanceOf query. To get a user's lent balance, query the **repo_token_cw20** contract (address in GetState) with the standard CW20 **Balance** message; when the CW20's pool_address is set, it returns underlying (scaled × liquidity_index). Use CW20 **TokenInfo** for total lent supply.

**GetBorrowerPosition** (`query/borrower_position.rs`)
Answers: “How much does this address owe?” The contract loads the borrower’s stored scaled debt and the effective reserve (indexes accrued to now). Underlying debt = scaled debt × borrow index. So the returned debt is the current amount owed in lending units, including accrued interest. Response includes both scaled and underlying.

**GetCollateralRequirements** (`query/collateral_requirements.rs`)
Answers: “How much collateral do I need to borrow this much?” Inputs: optional borrower (if they already have debt/collateral), the new loan amount, and the list of collateral asset ids to get minimums for. If the new loan amount is zero, the response is all zeros.

**Otherwise:** The contract fetches oracle prices for the lending denom and the requested collateral assets (and, if a borrower is given, their existing collateral denoms). It converts the new loan amount into a “value” (same units as prices). If a borrower is provided, it loads their current debt (effective reserve) and **existing collateral value** (same haircuts/oracle). **Total debt value** = existing debt value + new loan value. **Required total collateral value** (after haircuts) = total debt value ÷ margin rate. **Additional collateral value needed** = required total − existing collateral value (capped at zero).

**Per-asset minimums:** For each requested collateral asset, the minimum amount is **additional value needed** (or full required value when no borrower) divided by (price × haircut) for that asset, rounded up. So when a borrower is set, the amounts are “how much **more** of each asset to add.”

**Response:** **required_collateral_value_usd**, **additional_collateral_value_usd** (use for “you need $X more” or to combine assets), and **required**: (asset_id, min_amount) per asset. A UI can show total required, additional needed, and per-asset minimums.

---

## 7. Instantiate and migrate

- **Instantiate:** Validates config, stores contract state and a fresh reserve (indexes at 1, totals at zero, **`deficit_underlying`** at zero), sets cw2 metadata.
- **Migrate:** Confirms the contract id and bumps the on-chain version. Older reserve blobs without a deficit field load as **0** thanks to serde defaults. Ship a higher **`CONTRACT_VERSION`** than what is stored, then run **migrate** when you upgrade live code.

---

This walkthrough starts with the **models** (what the data means) and then ties them into storage, messages, rate logic, execute, and query so the whole flow is clear.
