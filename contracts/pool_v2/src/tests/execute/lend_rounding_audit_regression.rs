//! Regression for the **ceil-on-lend** audit scenario (see also repo-root
//! `ceil_rounding_drift_accumulates_until_lender_consumes_accrued_reserve.rs` for the old expected
//! failure mode).
//!
//! **Background:** If lend minted scaled units with `ceil(underlying / liquidity_index)`, some
//! deposits make `floor(minted_scaled × index) == deposit + 1`. The books then credit one extra
//! base unit of lender claim per such lend while the vault only received `deposit` coins. That
//! inflated `total_liquidity` vs actual cash and could block withdraws or pay lenders from fee
//! (`accrued_reserve`) balance. The contract now uses **floor** on lend; this test replays the
//! same *stress amounts* and asserts sound accounting.
//!
//! **Custody identity (what we assert each round):** Treat `contract_balance` as coins sitting in
//! the pool vault (we maintain it as a test-side shadow of deposits minus borrows plus repays).
//! `lhs = contract_balance + total_borrow` is “all lending-denom units accounted for in custody /
//! borrower hands.” `rhs = total_liquidity + accrued_reserve` is “aggregate book liabilities to
//! lenders and protocol.” In a sound pool, `rhs` must not exceed `lhs` (no phantom liabilities).
//!
//! **CW20:** Unit tests do not run a real repo token contract; we mock the scaled-balance query
//! the same way as `withdraw_tests`, using `reserve.total_scaled_liquidity` so the mock matches
//! reserve state for the final owner-on-behalf withdraw.

use crate::contract::execute;
use crate::instantiate::instantiate_contract;
use crate::model::error::{illegal_state, ContractError};
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::get_reserve_state_v1;
use crate::utils::{
    compute_effective_reserve, reserve_totals_and_cash_u128, scaled_to_underlying_liquidity,
};
use cosmwasm_std::testing::{message_info, mock_env};
use cosmwasm_std::{
    coin, ensure, from_json, to_json_binary, Addr, BankMsg, Coin, ContractResult, CosmosMsg,
    Decimal256, QuerierResult, SystemError, SystemResult, Uint128, Uint256, WasmQuery,
};
use cw20::BalanceResponse;
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use serde_json::{from_slice as json_from_slice, Value as JsonValue};
use std::collections::HashMap;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const BORROWER: &str = "tp1borrower";
const LENDING_DENOM: &str = "uylds.fcc";
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
const COLLATERAL_BTC: &str = "nbtc.figure.se";
const BTC_PRICE_USD: &str = "70000";
/// Sole lender: valid bech32 for `Withdraw { lender }` validation.
const LENDER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";

/// `ceil(underlying / liquidity_index)` in the same fixed-point shape as `rates` helpers. Only
/// used in this module to choose **probe** deposit sizes (historic ceil-on-lend bug); production
/// `Lend` uses floor only.
fn ceil_scaled_liquidity_for_probe(
    underlying: u128,
    liquidity_index: Decimal256,
) -> Result<u128, ContractError> {
    ensure!(
        !liquidity_index.is_zero(),
        illegal_state("liquidity_index is zero")
    );

    let underlying_d = Decimal256::from_ratio(Uint128::from(underlying), Uint128::from(1u128));
    let scaled_d = underlying_d.checked_div(liquidity_index)?;
    let atomics = scaled_d.atomics();
    let exp = Uint256::from(10u64).pow(18u32);
    let whole = atomics.checked_div(exp)?;
    let remainder = atomics.checked_rem(exp)?;
    let whole_ceil = if remainder.is_zero() {
        whole
    } else {
        whole + Uint256::from(1u64)
    };
    whole_ceil
        .to_string()
        .parse::<u128>()
        .map_err(|_| ContractError::IllegalStateError {
            message: "scaled liquidity overflow".to_string(),
        })
}

fn default_instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        contract_name: "pool-v2-demo".to_string(),
        description: "Test pool v2".to_string(),
        repo_token: RepoTokenConfig::Existing {
            repo_token_cw20_contract_address: REPO_TOKEN_CW20.to_string(),
        },
        lending_denom: Denom::new(LENDING_DENOM, 6u32),
        rate_params: RateParamsV1 {
            target_rate: Decimal256::from_str("0.09").unwrap(),
            min_rate: Decimal256::from_str("0.0325").unwrap(),
            max_rate: Decimal256::from_str("0.20").unwrap(),
            kink_utilization: Decimal256::from_str("0.90").unwrap(),
            reserve_factor: Decimal256::from_str("0.005").unwrap(),
            fee_model: Default::default(),
            flat_fee_apr: Decimal256::zero(),
            seconds_per_year: 31_536_000,
        },
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
            asset_id: COLLATERAL_BTC.to_string(),
            haircut: Some(Decimal256::percent(80)),
        }],
        commit_market_id: None,
        bad_debt_loss_allocation: Default::default(),
    }
}

fn price_entry(price: &str) -> AssetPriceResponseV1 {
    AssetPriceResponseV1 {
        price_usd: Decimal256::from_str(price).unwrap(),
        as_of_epoch_second: 0,
        expiration_epoch_seconds: u64::MAX,
    }
}

fn set_oracle_prices(
    querier: &mut provwasm_mocks::MockProvenanceQuerier,
    prices: PriceMapResponse,
) {
    let handler = move |query: &WasmQuery| -> QuerierResult {
        match query {
            WasmQuery::Smart { contract_addr, msg } => {
                if contract_addr.as_str() != ORACLE {
                    return SystemResult::Err(SystemError::NoSuchContract {
                        addr: contract_addr.to_string(),
                    });
                }
                match from_json::<PriceOracleQueryMsg>(msg) {
                    Ok(PriceOracleQueryMsg::GetPricesByAsset { assets: _ }) => {
                        SystemResult::Ok(ContractResult::Ok(to_json_binary(&prices).unwrap()))
                    }
                    _ => SystemResult::Err(SystemError::UnsupportedRequest {
                        kind: "unexpected oracle query".to_string(),
                    }),
                }
            }
            _ => SystemResult::Err(SystemError::UnsupportedRequest {
                kind: "expected WasmQuery::Smart".to_string(),
            }),
        }
    };
    querier.mock_querier.update_wasm(handler);
}

/// Mock repo token `scaled_balance` → `BalanceResponse`, scoped to `REPO_TOKEN_CW20` only (same
/// pattern as `withdraw_tests::mock_repo_scaled_balance`).
fn mock_repo_scaled_balance(
    querier: &mut provwasm_mocks::MockProvenanceQuerier,
    lender_scaled_balance: u128,
) {
    let balance = lender_scaled_balance;
    let handler = move |query: &WasmQuery| -> QuerierResult {
        match query {
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr.as_str() == REPO_TOKEN_CW20 =>
            {
                if let Ok(v) = json_from_slice::<JsonValue>(msg.as_slice()) {
                    if v.get("scaled_balance")
                        .and_then(|b| b.get("address"))
                        .and_then(|a| a.as_str())
                        .is_some()
                    {
                        return SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&BalanceResponse {
                                balance: Uint128::from(balance),
                            })
                            .unwrap(),
                        ));
                    }
                }
                SystemResult::Err(SystemError::UnsupportedRequest {
                    kind: "expected scaled_balance query".to_string(),
                })
            }
            _ => SystemResult::Err(SystemError::NoSuchContract {
                addr: "unknown".to_string(),
            }),
        }
    };
    querier.mock_querier.update_wasm(handler);
}

#[test]
fn lend_rounding_audit_scenario_does_not_inflate_liabilities_or_drain_reserve() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut env = mock_env();

    // --- Setup: pool + oracle, initial liquidity, then borrow half so utilization accrues indexes ---

    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        default_instantiate_msg(),
    )
    .expect("instantiate should succeed");

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_BTC.to_string(), price_entry(BTC_PRICE_USD));
    set_oracle_prices(&mut deps.querier, prices);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(LENDER), &[coin(10_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("first lend should succeed");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(250, COLLATERAL_BTC)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral should succeed");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(5_000_000),
        },
    )
    .expect("borrow should succeed");

    let rate_params = default_instantiate_msg().rate_params;
    // Vault cash after initial lend and borrow (no interest yet): 10M in, 5M borrowed out.
    let mut contract_balance: u128 = 10_000_000 - 5_000_000;

    // --- Stress rounds: same calendar cadence as the audit (30 days), same *choice* of lend size ---

    for round in 1u128..=5 {
        env.block.time = env.block.time.plus_seconds(30 * 24 * 3600);

        // Index the pool will see on the next execute (matches implicit accrual inside `Lend`).
        let projected =
            compute_effective_reserve(deps.as_ref().storage, env.block.time, &rate_params)
                .expect("compute_effective_reserve should succeed");
        let liquidity_index = projected.liquidity_index;

        // Smallest amount ≥ 1M where *hypothetical* ceil-mint would re-value to deposit+1 after
        // floor back to underlying. We only use this search to pick aggressive amounts; the pool
        // itself uses floor-on-lend (`underlying_to_scaled_liquidity`), not this ceil helper.
        let amount = (1_000_000u128..)
            .find(|&amt| {
                let scaled = ceil_scaled_liquidity_for_probe(amt, liquidity_index).unwrap();
                scaled_to_underlying_liquidity(scaled, liquidity_index).unwrap() == amt + 1
            })
            .expect("triggering amount should exist for this scenario");

        contract_balance += amount;

        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(LENDER), &[coin(amount, LENDING_DENOM)]),
            ExecuteMsg::Lend {},
        )
        .expect("lend should succeed");

        let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
        let (total_liquidity, total_borrow, _) = reserve_totals_and_cash_u128(&reserve).unwrap();
        let accrued_reserve: u128 = reserve.accrued_reserve;
        let lhs = contract_balance.saturating_add(total_borrow);
        let rhs = total_liquidity.saturating_add(accrued_reserve);

        // Buggy ceil-on-lend: rhs crept to lhs + 1 each round. Floor-on-lend: rhs must stay ≤ lhs.
        assert!(
            rhs <= lhs,
            "round {}: book liabilities {} must not exceed custody view {} (ceil-on-lend bug grew rhs by one per round)",
            round, rhs, lhs
        );
    }

    // --- Unwind borrow so sole lender can exit; track returned principal back into vault ---

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let (_, total_borrow, _) = reserve_totals_and_cash_u128(&reserve).unwrap();

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(total_borrow, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .expect("repay should succeed");
    contract_balance = contract_balance.saturating_add(total_borrow);

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let lender_scaled = reserve.total_scaled_liquidity;

    // Owner withdraw on behalf of lender queries repo CW20 for `lender`'s scaled balance; mock returns aggregate
    // scaled supply so it stays consistent with `reserve.total_scaled_liquidity`.
    mock_repo_scaled_balance(&mut deps.querier, lender_scaled);

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: None,
            commit_funds: None,
        },
    )
    .expect("full withdrawal should succeed");

    // --- Post-withdraw: vault must still cover booked protocol fees (old bug paid ~5 from fees) ---

    let send_amount = res
        .messages
        .iter()
        .find_map(|m| match &m.msg {
            CosmosMsg::Bank(BankMsg::Send { amount: coins, .. }) => {
                coins.first().map(|c: &Coin| c.amount.u128())
            }
            _ => None,
        })
        .expect("BankMsg::Send");

    let pool_residual_after = contract_balance.saturating_sub(send_amount);
    let reserve_after = get_reserve_state_v1(deps.as_ref().storage).unwrap();

    assert!(
        send_amount <= contract_balance,
        "withdraw must not exceed on-hand balance"
    );
    // `accrued_reserve` in state is the protocol fee accrual; coins for it should remain in the
    // vault after the lender is paid. Under the historical bug, `send_amount` was inflated so
    // residual dipped roughly five base units below this line.
    assert!(
        pool_residual_after >= reserve_after.accrued_reserve.saturating_sub(2),
        "pool residual after full withdraw ({pool_residual_after}) must cover accrued_reserve in storage ({}) \
         within dust tolerance; old bug left ~accrued−5",
        reserve_after.accrued_reserve
    );
    assert!(
        lender_scaled > 0,
        "sanity: lender should hold positive scaled supply"
    );
}
