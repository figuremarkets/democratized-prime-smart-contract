//! Integration tests that tie pool_v2 reserve state to a spreadsheet.
//!
//! **Spreadsheet:** Use values from `DP V2 Example.xlsx` (repo root) so that reserve totals
//! and events can be reconciled row-by-row.
//!
//! **Invariants:**
//! 1. **Assets − Liabilities = 0**: cash + total_borrow = total_liquidity (rounding is floor
//!    when converting scaled → underlying, so the identity holds in u128).
//! 2. After each event, total_liquidity and total_borrow match the expected values from the
//!    scenario (within 1 unit when interest accrues).

use crate::contract::{execute, query};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError as ModelContractError;
use crate::model::{
    CollateralAssetV1, Denom, FeeModelV1, RateParamsV1, ReserveStateV1, StateResponseV1,
};
use crate::msg::execute::Cw20ReceivePayload;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, RepoTokenConfig};
use crate::storage::get_scaled_borrow;
use crate::utils::{
    scaled_to_underlying_borrow, scaled_to_underlying_liquidity, underlying_to_scaled_liquidity,
};
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, ContractResult, Decimal256, Env, MemoryStorage,
    OwnedDeps, QuerierResult, SystemError, SystemResult, Timestamp, Uint128, WasmQuery,
};
use cw20::Cw20ReceiveMsg;
use cw20::{BalanceResponse, Cw20QueryMsg};
use democratized_prime_lib::price_oracle::model::AssetPriceResponseV1;
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

// --- Constants (tie to spreadsheet: DP V2 Example.xlsx) ---
const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
/// Lending denom: "u" prefix => 1 ylds.fcc = 10^6 uylds.fcc (6 decimals).
const LENDING_DENOM: &str = "uylds.fcc";
const LENDING_DENOM_PRECISION: u32 = 6;
/// Spreadsheet amounts are in display units ($1 = 1 ylds.fcc); 1 display unit = 10^6 base units (uylds.fcc).
const UNITS_PER_DISPLAY: u128 = 1_000_000;
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
/// Collateral denom: "nano" prefix => 1 BTC = 10^9 nbtc.figure.se (9 decimals).
const COLLATERAL_DENOM: &str = "nbtc.figure.se";
/// 1 BTC = $70,000; lending has 6 decimals => 1 nbtc = 70_000 * 10^6 / 10^9 = 70 lending base units.
const NBTC_PRICE_LENDING_BASE: &str = "70";
/// Event 12: price drop so D is liquidatable (LTV >= 90%). With 10 BTC and debt ~4.9e9, need value <= debt/0.9 => price <= 0.68 per nbtc.
const NBTC_PRICE_LIQUIDATION: &str = "0.68";

// Twelve-event spreadsheet: User A/B = lenders, User C/D = borrowers, User E = transfer recipient, User L = liquidator (contract owner).
const USER_A: &str = "tp1hwe54mzazqdh2786vghkfmkqu7j0mkx6te6gy8";
const USER_B: &str = "tp1wvefn22cq723u98f6mdqykf6w6avfckjzp6rtz";
const USER_C: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const USER_D: &str = "tp1w9p4tkctug2jyyx663f77x7e5cdry067z6xee4";
const USER_E: &str = "tp1cpcnkl3hpyv8sma7t3kyxzj23kjzuqypwhx3k0";
const USER_L: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c"; // liquidator = owner

/// Spreadsheet rate params: target 9%, min 3.25%, max 20%, kink 90%, reserve 0.5%, 31_536_000 s/year.
fn spreadsheet_rate_params() -> RateParamsV1 {
    RateParamsV1 {
        target_rate: Decimal256::from_str("0.09").unwrap(),
        min_rate: Decimal256::from_str("0.0325").unwrap(),
        max_rate: Decimal256::from_str("0.20").unwrap(),
        kink_utilization: Decimal256::from_str("0.90").unwrap(),
        reserve_factor: Decimal256::from_str("0.005").unwrap(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    }
}

/// Flat-fee spreadsheet params: same curve but protocol fee is a 50 bps spread off borrower APR.
fn spreadsheet_rate_params_flat_spread() -> RateParamsV1 {
    let mut p = spreadsheet_rate_params();
    p.fee_model = FeeModelV1::FlatBorrowSpread;
    p.flat_fee_apr = Decimal256::from_str("0.005").unwrap();
    p
}

fn default_instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        contract_name: "pool-v2-spreadsheet-test".to_string(),
        description: "Integration test tied to DP V2 Example.xlsx".to_string(),
        repo_token: RepoTokenConfig::Existing {
            repo_token_cw20_contract_address: REPO_TOKEN_CW20.to_string(),
        },
        lending_denom: Denom::new(LENDING_DENOM, LENDING_DENOM_PRECISION),
        rate_params: spreadsheet_rate_params(),
        lender_required_attrs: vec![],
        borrower_required_attrs: vec![],
        price_oracle_address: ORACLE.to_string(),
        max_borrower_collateral_types: 5,
        margin_rate: Decimal256::from_str("0.80").unwrap(),
        liquidation_rate: Decimal256::from_str("0.90").unwrap(),
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128),
        min_lend: Uint128::new(1),
        min_borrow: Uint128::new(1),
        supported_collateral_assets: vec![CollateralAssetV1 {
            asset_id: COLLATERAL_DENOM.to_string(),
            haircut: Some(Decimal256::percent(80)),
        }],
        commit_market_id: None,
        bad_debt_loss_allocation: Default::default(),
    }
}

fn flat_spread_instantiate_msg() -> InstantiateMsg {
    let mut msg = default_instantiate_msg();
    msg.rate_params = spreadsheet_rate_params_flat_spread();
    msg
}

fn price_entry(price: &str) -> AssetPriceResponseV1 {
    AssetPriceResponseV1 {
        price_usd: Decimal256::from_str(price).unwrap(),
        as_of_epoch_second: 0,
        expiration_epoch_seconds: u64::MAX,
    }
}

thread_local! {
    static ORACLE_PRICES: RefCell<Option<HashMap<String, AssetPriceResponseV1>>> =
        const { RefCell::new(None) };
}

fn set_oracle_prices(
    querier: &mut provwasm_mocks::MockProvenanceQuerier,
    prices: HashMap<String, AssetPriceResponseV1>,
) {
    ORACLE_PRICES.with(|p| *p.borrow_mut() = Some(prices.clone()));
    update_wasm_combined(querier);
}

fn update_wasm_combined(querier: &mut provwasm_mocks::MockProvenanceQuerier) {
    let handler = move |query: &WasmQuery| -> QuerierResult {
        match query {
            WasmQuery::Smart { contract_addr, msg } => {
                if contract_addr.as_str() == REPO_TOKEN_CW20 {
                    if let Ok(Cw20QueryMsg::Balance { address }) = from_json::<Cw20QueryMsg>(msg) {
                        let (a, b, e) = REPO_BALANCES.with(|r| *r.borrow());
                        let balance = if address == USER_A {
                            a
                        } else if address == USER_B {
                            b
                        } else if address == USER_E {
                            e
                        } else {
                            0
                        };
                        return SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&BalanceResponse {
                                balance: Uint128::from(balance),
                            })
                            .unwrap(),
                        ));
                    }
                }
                if contract_addr.as_str() == ORACLE {
                    if let Ok(PriceOracleQueryMsg::GetPricesByAsset { assets: _ }) =
                        from_json::<PriceOracleQueryMsg>(msg)
                    {
                        if let Some(prices) = ORACLE_PRICES.with(|p| p.borrow().clone()) {
                            return SystemResult::Ok(ContractResult::Ok(
                                to_json_binary(&prices).unwrap(),
                            ));
                        }
                    }
                }
                SystemResult::Err(SystemError::NoSuchContract {
                    addr: contract_addr.to_string(),
                })
            }
            _ => SystemResult::Err(SystemError::UnsupportedRequest {
                kind: "expected WasmQuery::Smart".to_string(),
            }),
        }
    };
    querier.mock_querier.update_wasm(handler);
}

/// Get total_liquidity and total_borrow in underlying u128 (same rounding as contract).
fn reserve_underlying(reserve: &ReserveStateV1) -> Result<(u128, u128), ModelContractError> {
    let liq =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)?;
    let bor = scaled_to_underlying_borrow(reserve.total_scaled_borrow, reserve.borrow_index)?;
    Ok((liq, bor))
}

/// Assert assets − liabilities = 0 including protocol reserve and optional **deficit_underlying**:
/// implied cash `total_liquidity + accrued_reserve - total_borrow - deficit` must be non-negative.
fn assert_assets_minus_liabilities_zero(
    total_liquidity_u128: u128,
    total_borrow_u128: u128,
    accrued_reserve_u128: u128,
    deficit_underlying_u128: u128,
    step_name: &str,
) {
    assert!(
        total_liquidity_u128
            .saturating_add(accrued_reserve_u128)
            >= total_borrow_u128.saturating_add(deficit_underlying_u128),
        "{}: assets - liabilities must balance; need liq + accrued >= bor + deficit; liq={} bor={} accrued_reserve={} deficit={}",
        step_name,
        total_liquidity_u128,
        total_borrow_u128,
        accrued_reserve_u128,
        deficit_underlying_u128
    );
}

/// Epsilon for Decimal256 comparison (in base units). ~10 base units ≈ 0.00001 display; allows index rounding.
fn epsilon_d256() -> Decimal256 {
    Decimal256::from_str("10").unwrap()
}

/// Assert two Decimal256 values are within epsilon (for matching spreadsheet with minimal rounding).
fn assert_decimal256_near(
    actual: Decimal256,
    expected: Decimal256,
    epsilon: Decimal256,
    step_and_label: &str,
) {
    let diff = if actual >= expected {
        actual.checked_sub(expected).unwrap_or(Decimal256::zero())
    } else {
        expected.checked_sub(actual).unwrap_or(Decimal256::zero())
    };
    assert!(
        diff < epsilon,
        "{}: actual {} vs expected {} (diff {} >= epsilon {})",
        step_and_label,
        actual,
        expected,
        diff,
        epsilon
    );
}

/// In flat spread mode we should always have: borrower_rate*U = lender_rate + flat_fee_apr*U.
fn assert_flat_spread_rate_split_identity(
    reserve: &ReserveStateV1,
    params: &RateParamsV1,
    label: &str,
) {
    let utilization = reserve.utilization().expect("utilization");
    let borrower_rate =
        crate::utils::borrower_rate_from_utilization(params, utilization).expect("borrower_rate");
    let lender_rate =
        crate::utils::lender_rate_from_utilization(params, utilization, borrower_rate)
            .expect("lender_rate");
    let borrower_flow = borrower_rate.checked_mul(utilization).expect("borrower*U");
    let protocol_flow = params
        .flat_fee_apr
        .checked_mul(utilization)
        .expect("flat_fee*U");
    let rhs = lender_rate
        .checked_add(protocol_flow)
        .expect("lender+protocol");
    let eps = Decimal256::from_str("0.0000000001").unwrap();
    assert_decimal256_near(borrower_flow, rhs, eps, label);
}

/// Underlying debt for a borrower in Decimal256 (scaled * borrow_index), no truncation.
fn borrower_underlying_decimal256(
    scaled: u128,
    borrow_index: Decimal256,
) -> Result<Decimal256, ModelContractError> {
    let scaled_d = Decimal256::from_ratio(Uint128::from(scaled), Uint128::from(1u128));
    scaled_d.checked_mul(borrow_index).map_err(Into::into)
}

thread_local! {
    static REPO_BALANCES: RefCell<(u128, u128, u128)> = const { RefCell::new((0, 0, 0)) };
}

/// Set mock CW20 repo token balances for USER_A, USER_B, USER_E (BalanceOf uses CW20 Balance query).
fn set_repo_token_balances(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    user_a_scaled: u128,
    user_b_scaled: u128,
    user_e_scaled: u128,
) {
    REPO_BALANCES.with(|r| *r.borrow_mut() = (user_a_scaled, user_b_scaled, user_e_scaled));
    update_wasm_combined(&mut deps.querier);
}

/// Assert repo token (CW20) scaled balance for `address` by querying the CW20 contract.
fn assert_repo_token_scaled_balance(
    deps: &OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    address: &str,
    expected_scaled: u128,
    step: &str,
) {
    let balance: BalanceResponse = deps
        .as_ref()
        .querier
        .query_wasm_smart(
            REPO_TOKEN_CW20,
            &Cw20QueryMsg::Balance {
                address: address.to_string(),
            },
        )
        .expect("CW20 Balance query");
    assert_eq!(
        balance.balance.u128(),
        expected_scaled,
        "{} {} repo token scaled balance",
        step,
        address
    );
}

/// Query GetBorrowerPosition for `address` and assert scaled_borrow and underlying_debt.
fn assert_user_borrow(
    deps: &OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    env: &Env,
    address: &str,
    expected_underlying_str: &str,
    reserve: &ReserveStateV1,
    eps: Decimal256,
    step: &str,
) {
    let bin = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::GetBorrowerPosition {
            address: address.to_string(),
        },
    )
    .expect("GetBorrowerPosition query");
    let res: serde_json::Value = from_json(bin).expect("GetBorrowerPosition response");
    let scaled_str = res["scaled_borrow"].as_str().expect("scaled_borrow");
    let underlying_str = res["underlying_debt"].as_str().expect("underlying_debt");
    let actual_scaled: u128 = scaled_str.parse().expect("scaled_borrow u128");
    let actual_underlying = Decimal256::from_str(underlying_str).unwrap();
    let expected_underlying = Decimal256::from_str(expected_underlying_str).unwrap();
    assert_decimal256_near(
        actual_underlying,
        expected_underlying,
        eps,
        &format!("{} {} underlying_debt (balanceOf)", step, address),
    );
    // Scaled should be consistent: scaled * borrow_index ≈ underlying
    let from_scaled =
        borrower_underlying_decimal256(actual_scaled, reserve.borrow_index).expect("borrower d256");
    assert_decimal256_near(
        from_scaled,
        expected_underlying,
        eps,
        &format!("{} {} scaled*index vs underlying", step, address),
    );
}

/// Assert reserve totals match expected (within `tolerance` to allow for accrued interest and rounding).
fn assert_reserve_ties_out_with_tolerance(
    reserve: &ReserveStateV1,
    expected_total_liquidity: u128,
    expected_total_borrow: u128,
    tolerance: u128,
    step_name: &str,
) {
    let (actual_liq, actual_bor) = reserve_underlying(reserve).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        actual_liq,
        actual_bor,
        reserve.accrued_reserve,
        reserve.deficit_underlying,
        step_name,
    );
    let liq_ok = actual_liq <= expected_total_liquidity + tolerance
        && actual_liq + tolerance >= expected_total_liquidity;
    let bor_ok = actual_bor <= expected_total_borrow + tolerance
        && actual_bor + tolerance >= expected_total_borrow;
    assert!(
        liq_ok,
        "{}: total_liquidity expected ~{} (tolerance {}) got {}",
        step_name, expected_total_liquidity, tolerance, actual_liq
    );
    assert!(
        bor_ok,
        "{}: total_borrow expected ~{} (tolerance {}) got {}",
        step_name, expected_total_borrow, tolerance, actual_bor
    );
}

/// Assert reserve totals match expected (within 1 unit for rounding, no time advance).
fn assert_reserve_ties_out(
    reserve: &ReserveStateV1,
    expected_total_liquidity: u128,
    expected_total_borrow: u128,
    step_name: &str,
) {
    assert_reserve_ties_out_with_tolerance(
        reserve,
        expected_total_liquidity,
        expected_total_borrow,
        1,
        step_name,
    );
}

/// Advance block time so the next execute sees interest accrual since last_updated_at.
fn advance_time(env: &mut Env, elapsed_seconds: u64) {
    env.block.time = Timestamp::from_seconds(env.block.time.seconds() + elapsed_seconds);
}

fn get_state_reserve(
    deps: &OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    env: &Env,
) -> ReserveStateV1 {
    let bin = query(deps.as_ref(), env.clone(), QueryMsg::GetState {}).expect("GetState");
    let state: StateResponseV1 = from_json(bin).expect("StateResponseV1");
    ReserveStateV1::from(state.reserve)
}

/// All 12 events from DP V2 Example.xlsx (Events table, rows 20–31).
///
/// Reserve (liq, bor) below are in **display units** (×10^6 = base); exact asserted values in the
/// test are in base units. Amounts reflect the test scenario (2 BTC / 10 BTC collateral, price 70
/// then 0.68 for event 12).
///
/// | #  | Participant | Action           | Amount (test)     | Reserve after (liq, bor) display ≈ |
/// |----|-------------|------------------|-------------------|-------------------------------------|
/// | 1  | User A      | Lend             | 1000              | (1000, 0)                           |
/// | 2  | User C      | Add collateral   | 2 BTC (2e9 nbtc)  | —                                   |
/// | 3  | User C      | Borrow           | 850               | (1000, 850)                         |
/// | 4  | User B      | Lend             | 5000              | (6000, 850)                         |
/// | 5  | User D      | Add collateral   | 10 BTC (10e9 nbtc)| —                                   |
/// | 6  | User D      | Borrow           | 4900              | (6000, 5750)                        |
/// | 7  | User C      | Pay loan         | 500               | (6000, 5252)                        |
/// | 8  | User C      | Remove collateral| 7 nbtc            | —                                   |
/// | 9  | User A      | Lend             | 100               | (6100, 5252)                        |
/// | 10 | User A      | Exit             | 500               | (5600, 5252)                        |
/// | 11 | User B      | Transfer         | 1000              | (5600, 5252)                        |
/// | 12 | User L      | Liquidation (D)  | repay 2991e6 base | (5600, 2261)                        |
///
/// Event 12: liquidator repays 2_991_286_533 base (contract min-repay for D's debt/collateral),
/// seizes 5.5e9 nbtc; total_borrow after ≈ 2261.49 display (2261490819 base).
///
/// Ensures assets − liabilities = 0 and reserve totals tie out after each event that changes reserve state.
///
/// **Time advancement:** Elapsed seconds between events are from DP V2 Example.xlsx (Date Time column B,
/// rows 20–31): (B_next - B_curr) * 86400.
///
/// **Expected totals:** Asserted in base units in the test; values were aligned with the scenario
/// (interest accrual, ceil/floor rounding). Tolerance allows for u128 truncation vs sheet floats.
const ELAPSED_SECONDS_BEFORE_EVENT: [u64; 11] = [
    5,     // before event 2  (1→2)
    5,     // before event 3  (2→3)
    5390,  // before event 4  (3→4)
    840,   // before event 5  (4→5)
    60,    // before event 6  (5→6)
    80100, // before event 7  (6→7)
    10,    // before event 8  (7→8)
    3590,  // before event 9  (8→9)
    1800,  // before event 10 (9→10)
    19800, // before event 11 (10→11)
    0,     // before event 12 (11→12)
];

/// Expected values are inlined at each assertion below (sheet: "Blockchain states by event").
///
/// **Scaled balances (readability):** The contract stores only `total_scaled_liquidity` on the
/// reserve; per-user liquidity is in the bank as repo token amounts (scaled). In this test we
/// reconstruct each lender's scaled balance from reserve totals so we can set mock bank balances
/// and assert BalanceOf. Rule: scaled amounts are **additive**—when a user supplies, we add
/// their new scaled amount to the running total. So we express each user's scaled balance as
/// named variables (e.g. `user_a_scaled`, `user_b_scaled`) built from "total scaled after event X"
/// and "delta in total scaled from event Y to Z", with short comments so it's clear *why* that
/// formula is that user's balance (e.g. "A supplied all of event 1→3, then added lent at 9").

#[test]
fn spreadsheet_events_liabilities_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut env = mock_env();

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(
        COLLATERAL_DENOM.to_string(),
        price_entry(NBTC_PRICE_LENDING_BASE), // 1 nbtc = 70 lending base units ($70k/BTC, 1 BTC = 10^9 nbtc)
    );
    set_oracle_prices(&mut deps.querier, prices.clone());

    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        default_instantiate_msg(),
    )
    .expect("instantiate");

    let reserve = get_state_reserve(&deps, &env);
    assert_reserve_ties_out(&reserve, 0, 0, "after instantiate");

    let eps = epsilon_d256();

    // Event 1: User A Lend 1000 (sheet $1,000 = 1000 * 10^6 base units)
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_A),
            &[coin(1000 * UNITS_PER_DISPLAY, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("event 1: lend");
    let reserve1 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 1: total_liquidity 1000, total_borrow 0 (in base units: * 10^6)
    assert_decimal256_near(
        reserve1.total_liquidity().unwrap(),
        Decimal256::from_str("1000000000").unwrap(),
        eps,
        "after event 1 total_liquidity",
    );
    assert_decimal256_near(
        reserve1.total_borrow().unwrap(),
        Decimal256::from_str("0").unwrap(),
        eps,
        "after event 1 total_borrow",
    );
    let (liq1, bor1) = reserve_underlying(&reserve1).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq1,
        bor1,
        reserve1.accrued_reserve,
        reserve1.deficit_underlying,
        "after event 1",
    );
    set_repo_token_balances(&mut deps, reserve1.total_scaled_liquidity, 0, 0);
    assert_repo_token_scaled_balance(
        &deps,
        USER_A,
        reserve1.total_scaled_liquidity,
        "after event 1",
    );

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[0]);

    // Event 2: User C Add collateral. 2 BTC = 2e9 nbtc; at 70 base/nbtc, value = 2e9*70*0.8 = 112e9, LTV 850e6/112e9 < 80%.
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_C),
            &[coin(2_000_000_000u128, COLLATERAL_DENOM)], // 2 BTC
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("event 2: add collateral");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[1]);

    // Event 3: User C Borrow 850 ($850 = 850 * 10^6 base units)
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(USER_C), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(850 * UNITS_PER_DISPLAY),
        },
    )
    .expect("event 3: borrow");
    let reserve3 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 3: total_liquidity 1000, total_borrow 850; User C debt 850 (base units)
    assert_decimal256_near(
        reserve3.total_liquidity().unwrap(),
        Decimal256::from_str("1000000000").unwrap(),
        eps,
        "after event 3 total_liquidity",
    );
    assert_decimal256_near(
        reserve3.total_borrow().unwrap(),
        Decimal256::from_str("850000000").unwrap(),
        eps,
        "after event 3 total_borrow",
    );
    let (liq3, bor3) = reserve_underlying(&reserve3).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq3,
        bor3,
        reserve3.accrued_reserve,
        reserve3.deficit_underlying,
        "after event 3",
    );
    let user_c_debt_3 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_C).unwrap(),
        reserve3.borrow_index,
    )
    .expect("User C debt");
    assert_decimal256_near(
        user_c_debt_3,
        Decimal256::from_str("850000000").unwrap(),
        eps,
        "after event 3 User C debt",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_C,
        "850000000",
        &reserve3,
        eps,
        "after event 3",
    );

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[2]);

    // Event 4: User B Lend 5000 ($5,000 = 5000 * 10^6 base units)
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_B),
            &[coin(5000 * UNITS_PER_DISPLAY, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("event 4: lend");
    let reserve4 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 4 (base units): liq 6000.012548*10^6, bor 850.012611*10^6; User C 850.012611*10^6
    assert_decimal256_near(
        reserve4.total_liquidity().unwrap(),
        Decimal256::from_str("6000012548").unwrap(),
        eps,
        "after event 4 total_liquidity",
    );
    assert_decimal256_near(
        reserve4.total_borrow().unwrap(),
        Decimal256::from_str("850012611").unwrap(),
        eps,
        "after event 4 total_borrow",
    );
    let (liq4, bor4) = reserve_underlying(&reserve4).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq4,
        bor4,
        reserve4.accrued_reserve,
        reserve4.deficit_underlying,
        "after event 4",
    );
    let user_c_debt_4 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_C).unwrap(),
        reserve4.borrow_index,
    )
    .expect("User C debt");
    assert_decimal256_near(
        user_c_debt_4,
        Decimal256::from_str("850012611").unwrap(),
        eps,
        "after event 4 User C debt",
    );
    // Scaled balances: A had all liquidity through event 3; B supplied at event 4.
    let user_a_scaled_after_4 = reserve3.total_scaled_liquidity;
    let user_b_scaled_after_4 = reserve4.total_scaled_liquidity - reserve3.total_scaled_liquidity;
    set_repo_token_balances(&mut deps, user_a_scaled_after_4, user_b_scaled_after_4, 0);
    assert_repo_token_scaled_balance(&deps, USER_A, user_a_scaled_after_4, "after event 4");
    assert_repo_token_scaled_balance(&deps, USER_B, user_b_scaled_after_4, "after event 4");
    assert_user_borrow(
        &deps,
        &env,
        USER_C,
        "850012611",
        &reserve4,
        eps,
        "after event 4",
    );

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[3]);

    // Event 5: User D Add collateral. 10 BTC = 10e9 nbtc; at 70 base/nbtc, value = 560e9, LTV 4900e6/560e9 < 80%.
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_D),
            &[coin(10_000_000_000u128, COLLATERAL_DENOM)], // 10 BTC
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("event 5: add collateral");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[4]);

    // Event 6: User D Borrow 4900 ($4,900 = 4900 * 10^6 base units)
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(USER_D), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(4900 * UNITS_PER_DISPLAY),
        },
    )
    .expect("event 6: borrow");
    let reserve6 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 6 (base units)
    assert_decimal256_near(
        reserve6.total_liquidity().unwrap(),
        Decimal256::from_str("6000013552").unwrap(),
        eps,
        "after event 6 total_liquidity",
    );
    assert_decimal256_near(
        reserve6.total_borrow().unwrap(),
        Decimal256::from_str("5750013620").unwrap(),
        eps,
        "after event 6 total_borrow",
    );
    let (liq6, bor6) = reserve_underlying(&reserve6).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq6,
        bor6,
        reserve6.accrued_reserve,
        reserve6.deficit_underlying,
        "after event 6",
    );
    let user_c_debt_6 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_C).unwrap(),
        reserve6.borrow_index,
    )
    .expect("User C debt");
    assert_decimal256_near(
        user_c_debt_6,
        Decimal256::from_str("850013620").unwrap(),
        eps,
        "after event 6 User C debt",
    );
    let user_d_debt_6 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_D).unwrap(),
        reserve6.borrow_index,
    )
    .expect("User D debt");
    assert_decimal256_near(
        user_d_debt_6,
        Decimal256::from_str("4900000000").unwrap(),
        eps,
        "after event 6 User D debt",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_C,
        "850013620",
        &reserve6,
        eps,
        "after event 6",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_D,
        "4900000000",
        &reserve6,
        eps,
        "after event 6",
    );

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[5]);

    // Event 7: User C Pay loan 500 ($500 = 500 * 10^6 base units)
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_C),
            &[coin(500 * UNITS_PER_DISPLAY, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .expect("event 7: repay");
    let reserve7 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 7 (base units)
    assert_decimal256_near(
        reserve7.total_liquidity().unwrap(),
        Decimal256::from_str("6002253865").unwrap(),
        eps,
        "after event 7 total_liquidity",
    );
    assert_decimal256_near(
        reserve7.total_borrow().unwrap(),
        Decimal256::from_str("5252265189").unwrap(),
        eps,
        "after event 7 total_borrow",
    );
    let (liq7, bor7) = reserve_underlying(&reserve7).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq7,
        bor7,
        reserve7.accrued_reserve,
        reserve7.deficit_underlying,
        "after event 7",
    );
    let user_c_debt_7 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_C).unwrap(),
        reserve7.borrow_index,
    )
    .expect("User C debt");
    assert_decimal256_near(
        user_c_debt_7,
        Decimal256::from_str("350346464").unwrap(),
        eps,
        "after event 7 User C debt",
    );
    let user_d_debt_7 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_D).unwrap(),
        reserve7.borrow_index,
    )
    .expect("User D debt");
    assert_decimal256_near(
        user_d_debt_7,
        Decimal256::from_str("4901918726").unwrap(),
        eps,
        "after event 7 User D debt",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_C,
        "350346464",
        &reserve7,
        eps,
        "after event 7",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_D,
        "4901918726",
        &reserve7,
        eps,
        "after event 7",
    );

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[6]);

    // Event 8: User C Remove collateral (7 nbtc; negligible vs 2 BTC).
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(USER_C), &[]),
        ExecuteMsg::RemoveCollateral {
            to_remove: BTreeMap::from([(COLLATERAL_DENOM.to_string(), Uint128::new(7))]),
        },
    )
    .expect("event 8: remove collateral");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[7]);

    // Event 9: User A Lend 100 ($100 = 100 * 10^6 base units)
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_A),
            &[coin(100 * UNITS_PER_DISPLAY, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("event 9: lend");
    let reserve9 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 9 (base units)
    assert_decimal256_near(
        reserve9.total_liquidity().unwrap(),
        Decimal256::from_str("6102306606").unwrap(),
        eps,
        "after event 9 total_liquidity",
    );
    assert_decimal256_near(
        reserve9.total_borrow().unwrap(),
        Decimal256::from_str("5252318195").unwrap(),
        eps,
        "after event 9 total_borrow",
    );
    let (liq9, bor9) = reserve_underlying(&reserve9).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq9,
        bor9,
        reserve9.accrued_reserve,
        reserve9.deficit_underlying,
        "after event 9",
    );
    let user_c_debt_9 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_C).unwrap(),
        reserve9.borrow_index,
    )
    .expect("User C debt");
    assert_decimal256_near(
        user_c_debt_9,
        Decimal256::from_str("350350000").unwrap(),
        eps,
        "after event 9 User C debt",
    );
    let user_d_debt_9 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_D).unwrap(),
        reserve9.borrow_index,
    )
    .expect("User D debt");
    assert_decimal256_near(
        user_d_debt_9,
        Decimal256::from_str("4901968196").unwrap(),
        eps,
        "after event 9 User D debt",
    );
    // A had balance through event 3, then supplied 100 at event 9 (total scaled increased reserve7→reserve9).
    let user_a_scaled_after_9 = reserve3.total_scaled_liquidity
        + (reserve9.total_scaled_liquidity - reserve7.total_scaled_liquidity);
    let user_b_scaled_after_9 = user_b_scaled_after_4; // B unchanged
    set_repo_token_balances(&mut deps, user_a_scaled_after_9, user_b_scaled_after_9, 0);
    assert_repo_token_scaled_balance(&deps, USER_A, user_a_scaled_after_9, "after event 9");
    assert_repo_token_scaled_balance(&deps, USER_B, user_b_scaled_after_9, "after event 9");
    assert_user_borrow(
        &deps,
        &env,
        USER_C,
        "350350000",
        &reserve9,
        eps,
        "after event 9",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_D,
        "4901968196",
        &reserve9,
        eps,
        "after event 9",
    );

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[8]);

    // Event 10: User A Exit 500 ($500 = 500 * 10^6 base units; withdraw via CW20 Receive)
    let withdraw_amount = 500 * UNITS_PER_DISPLAY;
    let scaled_10 =
        underlying_to_scaled_liquidity(withdraw_amount, reserve9.liquidity_index).unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: USER_A.to_string(),
            amount: Uint128::from(scaled_10),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_amount),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .expect("event 10: exit");
    let reserve10 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 10 (base units)
    assert_decimal256_near(
        reserve10.total_liquidity().unwrap(),
        Decimal256::from_str("5602332704").unwrap(),
        eps,
        "after event 10 total_liquidity",
    );
    assert_decimal256_near(
        reserve10.total_borrow().unwrap(),
        Decimal256::from_str("5252344424").unwrap(),
        eps,
        "after event 10 total_borrow",
    );
    let (liq10, bor10) = reserve_underlying(&reserve10).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq10,
        bor10,
        reserve10.accrued_reserve,
        reserve10.deficit_underlying,
        "after event 10",
    );
    let user_c_debt_10 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_C).unwrap(),
        reserve10.borrow_index,
    )
    .expect("User C debt");
    assert_decimal256_near(
        user_c_debt_10,
        Decimal256::from_str("350351751").unwrap(),
        eps,
        "after event 10 User C debt",
    );
    let user_d_debt_10 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_D).unwrap(),
        reserve10.borrow_index,
    )
    .expect("User D debt");
    assert_decimal256_near(
        user_d_debt_10,
        Decimal256::from_str("4901992675").unwrap(),
        eps,
        "after event 10 User D debt",
    );
    // A withdrew 500 display; total scaled dropped by (reserve9 - reserve10), all from A.
    let scaled_withdrawn_by_a_10 =
        reserve9.total_scaled_liquidity - reserve10.total_scaled_liquidity;
    let user_a_scaled_after_10 = user_a_scaled_after_9 - scaled_withdrawn_by_a_10;
    let user_b_scaled_after_10 = user_b_scaled_after_9; // B unchanged
    set_repo_token_balances(&mut deps, user_a_scaled_after_10, user_b_scaled_after_10, 0);
    assert_repo_token_scaled_balance(&deps, USER_A, user_a_scaled_after_10, "after event 10");
    assert_repo_token_scaled_balance(&deps, USER_B, user_b_scaled_after_10, "after event 10");
    assert_user_borrow(
        &deps,
        &env,
        USER_C,
        "350351751",
        &reserve10,
        eps,
        "after event 10",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_D,
        "4901992675",
        &reserve10,
        eps,
        "after event 10",
    );

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[9]);

    // Event 11: User B Transfer 1000 ($1,000 repo token) to User E via CW20 Receive — no reserve change
    let transfer_amount = 1000 * UNITS_PER_DISPLAY;
    let scaled_11 =
        underlying_to_scaled_liquidity(transfer_amount, reserve10.liquidity_index).unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: USER_B.to_string(),
            amount: Uint128::from(scaled_11),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: USER_E.to_string(),
                amount: Uint128::new(transfer_amount),
            })
            .unwrap(),
        }),
    )
    .expect("event 11: transfer");
    let reserve11 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 11 (base units)
    assert_decimal256_near(
        reserve11.total_liquidity().unwrap(),
        Decimal256::from_str("5602763465").unwrap(),
        eps,
        "after event 11 total_liquidity",
    );
    assert_decimal256_near(
        reserve11.total_borrow().unwrap(),
        Decimal256::from_str("5252777351").unwrap(),
        eps,
        "after event 11 total_borrow",
    );
    let (liq11, bor11) = reserve_underlying(&reserve11).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq11,
        bor11,
        reserve11.accrued_reserve,
        reserve11.deficit_underlying,
        "after event 11",
    );
    let user_c_debt_11 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_C).unwrap(),
        reserve11.borrow_index,
    )
    .expect("User C debt");
    assert_decimal256_near(
        user_c_debt_11,
        Decimal256::from_str("350380629").unwrap(),
        eps,
        "after event 11 User C debt",
    );
    let user_d_debt_11 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_D).unwrap(),
        reserve11.borrow_index,
    )
    .expect("User D debt");
    assert_decimal256_near(
        user_d_debt_11,
        Decimal256::from_str("4902396723").unwrap(),
        eps,
        "after event 11 User D debt",
    );
    // B transferred 1000 display (underlying) to E; convert to scaled at current index.
    let scaled_sent_b_to_e_11 =
        underlying_to_scaled_liquidity(1000 * UNITS_PER_DISPLAY, reserve11.liquidity_index)
            .expect("scaled for transfer");
    let user_a_scaled_after_11 = user_a_scaled_after_10; // A unchanged
    let user_b_scaled_after_11 = user_b_scaled_after_10 - scaled_sent_b_to_e_11;
    let user_e_scaled_after_11 = scaled_sent_b_to_e_11;
    set_repo_token_balances(
        &mut deps,
        user_a_scaled_after_11,
        user_b_scaled_after_11,
        user_e_scaled_after_11,
    );
    assert_repo_token_scaled_balance(&deps, USER_A, user_a_scaled_after_11, "after event 11");
    assert_repo_token_scaled_balance(&deps, USER_B, user_b_scaled_after_11, "after event 11");
    assert_repo_token_scaled_balance(&deps, USER_E, user_e_scaled_after_11, "after event 11");
    assert_user_borrow(
        &deps,
        &env,
        USER_C,
        "350380629",
        &reserve11,
        eps,
        "after event 11",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_D,
        "4902396723",
        &reserve11,
        eps,
        "after event 11",
    );

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[10]);

    // Event 12: User L Liquidation — price 0.68 so D's 10 BTC has value 5.44e9, LTV >= 90%. Repay repay_12; seize nbtc so market value in [repay, repay*1.02].
    // Market value = amount * 0.68 => amount in [repay_12/0.68, repay_12*1.02/0.68] ≈ 4.4e9–4.49e9 nbtc.
    prices.insert(
        COLLATERAL_DENOM.to_string(),
        price_entry(NBTC_PRICE_LIQUIDATION),
    );
    set_oracle_prices(&mut deps.querier, prices);
    // Min repay to bring D's LTV to healthy (contract enforces this)
    let repay_12 = 2_991_286_533u128;
    let seize_12 = 4_400_000_000u128; // nbtc so market value 4.4e9*0.68 ≈ 2.992e9 (in [repay_12, repay_12*1.02])
    let mut collateral_to_seize = BTreeMap::new();
    collateral_to_seize.insert(COLLATERAL_DENOM.to_string(), Uint128::new(seize_12));
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(USER_L), &[coin(repay_12, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: USER_D.to_string(),
            collateral_to_seize,
        },
    )
    .expect("event 12: liquidation");
    let reserve12 = get_state_reserve(&deps, &env);
    // Spreadsheet after event 12 (base units)
    assert_decimal256_near(
        reserve12.total_liquidity().unwrap(),
        Decimal256::from_str("5602763465").unwrap(),
        eps,
        "after event 12 total_liquidity",
    );
    assert_decimal256_near(
        reserve12.total_borrow().unwrap(),
        Decimal256::from_str("2261490819").unwrap(),
        eps,
        "after event 12 total_borrow",
    );
    let (liq12, bor12) = reserve_underlying(&reserve12).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq12,
        bor12,
        reserve12.accrued_reserve,
        reserve12.deficit_underlying,
        "after event 12",
    );
    let user_c_debt_12 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_C).unwrap(),
        reserve12.borrow_index,
    )
    .expect("User C debt");
    assert_decimal256_near(
        user_c_debt_12,
        Decimal256::from_str("350380627").unwrap(),
        eps,
        "after event 12 User C debt",
    );
    let user_d_debt_12 = borrower_underlying_decimal256(
        get_scaled_borrow(deps.as_ref().storage, USER_D).unwrap(),
        reserve12.borrow_index,
    )
    .expect("User D debt");
    assert_decimal256_near(
        user_d_debt_12,
        Decimal256::from_str("1911110190").unwrap(),
        eps,
        "after event 12 User D debt",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_C,
        "350380627",
        &reserve12,
        eps,
        "after event 12",
    );
    assert_user_borrow(
        &deps,
        &env,
        USER_D,
        "1911110190",
        &reserve12,
        eps,
        "after event 12",
    );

    let reserve = get_state_reserve(&deps, &env);
    let (liq, bor) = reserve_underlying(&reserve).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq,
        bor,
        reserve.accrued_reserve,
        reserve.deficit_underlying,
        "final (12 events)",
    );
}

#[test]
fn spreadsheet_events_liabilities_zero_flat_spread_model() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut env = mock_env();

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(
        COLLATERAL_DENOM.to_string(),
        price_entry(NBTC_PRICE_LENDING_BASE),
    );
    set_oracle_prices(&mut deps.querier, prices.clone());

    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        flat_spread_instantiate_msg(),
    )
    .expect("instantiate");

    let rate_params = spreadsheet_rate_params_flat_spread();

    // Event 1: User A Lend 1000
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_A),
            &[coin(1000 * UNITS_PER_DISPLAY, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("event 1: lend");
    let reserve1 = get_state_reserve(&deps, &env);
    let (liq1, bor1) = reserve_underlying(&reserve1).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq1,
        bor1,
        reserve1.accrued_reserve,
        reserve1.deficit_underlying,
        "flat spread after event 1",
    );
    assert_flat_spread_rate_split_identity(&reserve1, &rate_params, "flat spread event 1 split");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[0]);

    // Event 2: User C Add collateral
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_C),
            &[coin(2_000_000_000u128, COLLATERAL_DENOM)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("event 2: add collateral");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[1]);

    // Event 3: User C Borrow 850
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(USER_C), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(850 * UNITS_PER_DISPLAY),
        },
    )
    .expect("event 3: borrow");
    let reserve3 = get_state_reserve(&deps, &env);
    let (liq3, bor3) = reserve_underlying(&reserve3).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq3,
        bor3,
        reserve3.accrued_reserve,
        reserve3.deficit_underlying,
        "flat spread after event 3",
    );
    assert_flat_spread_rate_split_identity(&reserve3, &rate_params, "flat spread event 3 split");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[2]);

    // Event 4: User B Lend 5000
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_B),
            &[coin(5000 * UNITS_PER_DISPLAY, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("event 4: lend");
    let reserve4 = get_state_reserve(&deps, &env);
    let (liq4, bor4) = reserve_underlying(&reserve4).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq4,
        bor4,
        reserve4.accrued_reserve,
        reserve4.deficit_underlying,
        "flat spread after event 4",
    );
    assert_flat_spread_rate_split_identity(&reserve4, &rate_params, "flat spread event 4 split");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[3]);

    // Event 5: User D Add collateral
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_D),
            &[coin(10_000_000_000u128, COLLATERAL_DENOM)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("event 5: add collateral");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[4]);

    // Event 6: User D Borrow 4900
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(USER_D), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(4900 * UNITS_PER_DISPLAY),
        },
    )
    .expect("event 6: borrow");
    let reserve6 = get_state_reserve(&deps, &env);
    let (liq6, bor6) = reserve_underlying(&reserve6).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq6,
        bor6,
        reserve6.accrued_reserve,
        reserve6.deficit_underlying,
        "flat spread after event 6",
    );
    assert_flat_spread_rate_split_identity(&reserve6, &rate_params, "flat spread event 6 split");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[5]);

    // Event 7: User C Repay 500
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_C),
            &[coin(500 * UNITS_PER_DISPLAY, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .expect("event 7: repay");
    let reserve7 = get_state_reserve(&deps, &env);
    let (liq7, bor7) = reserve_underlying(&reserve7).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq7,
        bor7,
        reserve7.accrued_reserve,
        reserve7.deficit_underlying,
        "flat spread after event 7",
    );
    assert_flat_spread_rate_split_identity(&reserve7, &rate_params, "flat spread event 7 split");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[6]);

    // Event 8: User C Remove collateral
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(USER_C), &[]),
        ExecuteMsg::RemoveCollateral {
            to_remove: BTreeMap::from([(COLLATERAL_DENOM.to_string(), Uint128::new(7))]),
        },
    )
    .expect("event 8: remove collateral");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[7]);

    // Event 9: User A Lend 100
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(USER_A),
            &[coin(100 * UNITS_PER_DISPLAY, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("event 9: lend");
    let reserve9 = get_state_reserve(&deps, &env);
    let (liq9, bor9) = reserve_underlying(&reserve9).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq9,
        bor9,
        reserve9.accrued_reserve,
        reserve9.deficit_underlying,
        "flat spread after event 9",
    );
    assert_flat_spread_rate_split_identity(&reserve9, &rate_params, "flat spread event 9 split");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[8]);

    // Event 10: User A Exit 500
    let scaled_10 =
        underlying_to_scaled_liquidity(500 * UNITS_PER_DISPLAY, reserve9.liquidity_index).unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: USER_A.to_string(),
            amount: Uint128::from(scaled_10),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(500 * UNITS_PER_DISPLAY),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .expect("event 10: exit");
    let reserve10 = get_state_reserve(&deps, &env);
    let (liq10, bor10) = reserve_underlying(&reserve10).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq10,
        bor10,
        reserve10.accrued_reserve,
        reserve10.deficit_underlying,
        "flat spread after event 10",
    );
    assert_flat_spread_rate_split_identity(&reserve10, &rate_params, "flat spread event 10 split");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[9]);

    // Event 11: User B Transfer 1000
    let scaled_11 =
        underlying_to_scaled_liquidity(1000 * UNITS_PER_DISPLAY, reserve10.liquidity_index)
            .unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: USER_B.to_string(),
            amount: Uint128::from(scaled_11),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: USER_E.to_string(),
                amount: Uint128::new(1000 * UNITS_PER_DISPLAY),
            })
            .unwrap(),
        }),
    )
    .expect("event 11: transfer");
    let reserve11 = get_state_reserve(&deps, &env);
    let (liq11, bor11) = reserve_underlying(&reserve11).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq11,
        bor11,
        reserve11.accrued_reserve,
        reserve11.deficit_underlying,
        "flat spread after event 11",
    );
    assert_flat_spread_rate_split_identity(&reserve11, &rate_params, "flat spread event 11 split");

    advance_time(&mut env, ELAPSED_SECONDS_BEFORE_EVENT[10]);

    // Event 12: User L Liquidation
    prices.insert(
        COLLATERAL_DENOM.to_string(),
        price_entry(NBTC_PRICE_LIQUIDATION),
    );
    set_oracle_prices(&mut deps.querier, prices);
    let repay_12 = 2_991_286_750u128;
    let seize_12 = 4_400_000_000u128;
    let mut collateral_to_seize = BTreeMap::new();
    collateral_to_seize.insert(COLLATERAL_DENOM.to_string(), Uint128::new(seize_12));
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(USER_L), &[coin(repay_12, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: USER_D.to_string(),
            collateral_to_seize,
        },
    )
    .expect("event 12: liquidation");
    let reserve12 = get_state_reserve(&deps, &env);
    let (liq12, bor12) = reserve_underlying(&reserve12).expect("reserve_underlying");
    assert_assets_minus_liabilities_zero(
        liq12,
        bor12,
        reserve12.accrued_reserve,
        reserve12.deficit_underlying,
        "flat spread after event 12",
    );
    assert_flat_spread_rate_split_identity(&reserve12, &rate_params, "flat spread event 12 split");
}
