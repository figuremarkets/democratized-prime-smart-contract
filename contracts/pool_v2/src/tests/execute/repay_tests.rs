//! Tests for Repay execute: success (partial, full, excess refunded); failures for no funds,
//! wrong denom, two coins, no borrow, and zero amount.

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_SCALED_AMOUNT};
use crate::contract::execute;
use crate::execute::repay::ACTION;
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{get_contract_state_v1, get_reserve_state_v1, get_scaled_borrow};
use crate::tests::reserve_invariant::assert_reserve_assets_liabilities_tie_out_with_tolerance;
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use crate::utils::{
    compute_effective_reserve, scaled_to_underlying_borrow, scaled_to_underlying_liquidity,
};
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256,
    QuerierResult, SystemError, SystemResult, Timestamp, Uint128, WasmQuery,
};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use std::collections::HashMap;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const BORROWER: &str = "tp1borrower";
const LENDING_DENOM: &str = "uylds.fcc";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
const COLLATERAL_BTC: &str = "nbtc.figure.se";
const BTC_PRICE_USD: &str = "70000";

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
            seconds_per_year: 31_536_000,
        },
        lender_required_attrs: vec![],
        borrower_required_attrs: vec![],
        price_oracle_address: ORACLE.to_string(),
        max_borrower_collateral_types: 5,
        margin_rate: Decimal256::from_str("0.80").unwrap(),
        liquidation_rate: Decimal256::from_str("0.90").unwrap(),
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128), // 2%
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

/// Borrower has borrowed 10M; use for repay tests.
fn setup_borrower_with_debt() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
    u128,
) {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let msg = default_instantiate_msg();
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate should succeed");

    let lend_amount = 100_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(lend_amount, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend should succeed");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(250, COLLATERAL_BTC)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral should succeed");

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_BTC.to_string(), price_entry(BTC_PRICE_USD));
    set_oracle_prices(&mut deps.querier, prices);

    let borrow_amount = 10_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(borrow_amount),
        },
    )
    .expect("borrow should succeed");

    (deps, env, borrow_amount)
}

/// Allow drift from scaled↔underlying floor/ceil and index truncation (see tests::reserve_invariant).
const TOLERANCE_BASE_UNITS: u128 = 10;

#[test]
fn repay_succeeds_partial_and_decreases_debt() {
    let (mut deps, env, _) = setup_borrower_with_debt();
    let reserve_before = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let liq_before = scaled_to_underlying_liquidity(
        reserve_before.total_scaled_liquidity,
        reserve_before.liquidity_index,
    )
    .unwrap();
    let bor_before = scaled_to_underlying_borrow(
        reserve_before.total_scaled_borrow,
        reserve_before.borrow_index,
    )
    .unwrap();
    let implied_before = liq_before
        .saturating_add(reserve_before.accrued_reserve)
        .saturating_sub(bor_before);
    let scaled_before = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    let repay_amount = 3_000_000u128;
    let expected_implied_after = implied_before.saturating_add(repay_amount);

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(repay_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .expect("repay should succeed");

    assert_eq!(res.messages.len(), 0);
    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].key, "borrower");
    assert_eq!(res.attributes[1].value, BORROWER);
    assert_eq!(res.attributes[2].key, "amount");
    assert_eq!(res.attributes[2].value, repay_amount.to_string());

    let scaled_after = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(
        res.attributes[3].value,
        (scaled_before - scaled_after).to_string()
    );
    assert!(scaled_after > 0);
    assert!(scaled_after < scaled_before);
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_reserve_assets_liabilities_tie_out_with_tolerance(
        deps.as_ref().storage,
        "after repay (partial)",
        Some(expected_implied_after),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

#[test]
fn repay_succeeds_full_payoff() {
    let (mut deps, env, borrow_amount) = setup_borrower_with_debt();
    let scaled_before = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    let reserve_before = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let liq_before = scaled_to_underlying_liquidity(
        reserve_before.total_scaled_liquidity,
        reserve_before.liquidity_index,
    )
    .unwrap();
    let bor_before = scaled_to_underlying_borrow(
        reserve_before.total_scaled_borrow,
        reserve_before.borrow_index,
    )
    .unwrap();
    let implied_before = liq_before
        .saturating_add(reserve_before.accrued_reserve)
        .saturating_sub(bor_before);
    let repay_amount = borrow_amount; // exact payoff (no time advance in mock, debt ≈ borrow_amount)
    let expected_implied_after = implied_before.saturating_add(repay_amount);

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(repay_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .expect("repay should succeed");

    assert_eq!(res.messages.len(), 0);
    assert_eq!(res.attributes[2].value, repay_amount.to_string());
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[3].value, scaled_before.to_string());
    let scaled_after = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(scaled_after, 0);
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_reserve_assets_liabilities_tie_out_with_tolerance(
        deps.as_ref().storage,
        "after repay (full)",
        Some(expected_implied_after),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

/// Full repayment after interest has accrued: without the double-floor fix, scaled_repay =
/// floor(debt_underlying/borrow_index) can be < scaled_debt, leaving irremovable dust. This test
/// advances time so borrow_index > 1, then repays amount >= debt_underlying and asserts
/// scaled_borrow becomes 0 (and borrower can remove all collateral).
#[test]
fn repay_full_after_interest_accrual_clears_scaled_debt() {
    let (mut deps, mut env, borrow_amount) = setup_borrower_with_debt();
    // Advance time so borrow index grows (e.g. 1 year at min_rate ~3.25% -> index ~1.0325).
    const SECONDS_PER_YEAR: u64 = 31_536_000;
    env.block.time = Timestamp::from_seconds(env.block.time.seconds() + SECONDS_PER_YEAR);

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let reserve =
        compute_effective_reserve(deps.as_ref().storage, env.block.time, &contract.rate_params)
            .unwrap();
    let scaled_debt = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    let debt_underlying = scaled_to_underlying_borrow(scaled_debt, reserve.borrow_index).unwrap();
    assert!(
        reserve.borrow_index > Decimal256::one(),
        "index should grow after time advance"
    );
    assert!(
        debt_underlying >= borrow_amount,
        "debt should accrue interest"
    );

    // Repay full debt (user sends at least debt_underlying).
    let repay_amount = debt_underlying;
    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(repay_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .expect("repay should succeed");

    assert_eq!(res.attributes[2].value, repay_amount.to_string());
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[3].value, scaled_debt.to_string());
    let scaled_after = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(
        scaled_after, 0,
        "full repayment must clear all scaled debt (no dust); scaled_debt was {}, borrow_index {}",
        scaled_debt, reserve.borrow_index
    );
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
}

#[test]
fn repay_succeeds_excess_refunded() {
    let (mut deps, env, borrow_amount) = setup_borrower_with_debt();
    let scaled_before = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    let reserve_before = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let liq_before = scaled_to_underlying_liquidity(
        reserve_before.total_scaled_liquidity,
        reserve_before.liquidity_index,
    )
    .unwrap();
    let bor_before = scaled_to_underlying_borrow(
        reserve_before.total_scaled_borrow,
        reserve_before.borrow_index,
    )
    .unwrap();
    let implied_before = liq_before
        .saturating_add(reserve_before.accrued_reserve)
        .saturating_sub(bor_before);
    let sent = borrow_amount + 2_000_000u128;
    let expected_implied_after = implied_before.saturating_add(borrow_amount); // net cash in = debt paid off

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(sent, LENDING_DENOM)]),
        ExecuteMsg::Repay {},
    )
    .expect("repay should succeed");

    assert_eq!(res.messages.len(), 1);
    let msg = &res.messages[0].msg;
    match msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), BORROWER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, LENDING_DENOM);
            assert!(amount[0].amount.u128() >= 2_000_000);
        }
        _ => panic!("expected BankMsg::Send, got {:?}", msg),
    }
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[3].value, scaled_before.to_string());
    let scaled_after = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(scaled_after, 0);
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_reserve_assets_liabilities_tie_out_with_tolerance(
        deps.as_ref().storage,
        "after repay (excess)",
        Some(expected_implied_after),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

#[test]
fn repay_fails_no_funds() {
    let (mut deps, env, _) = setup_borrower_with_debt();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Repay {},
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Exactly one coin must be sent"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn repay_fails_wrong_denom() {
    let (mut deps, env, _) = setup_borrower_with_debt();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(1_000_000, "wrong.denom")],
        ),
        ExecuteMsg::Repay {},
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Expected denom"));
            assert!(message.contains(LENDING_DENOM));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn repay_fails_two_coins() {
    let (mut deps, env, _) = setup_borrower_with_debt();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(1_000_000, LENDING_DENOM), coin(500_000, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Exactly one coin must be sent"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn repay_fails_no_borrow() {
    let (mut deps, env, _) = setup_borrower_with_debt();
    // Use a different user who never borrowed
    let other = "tp1other";

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(other), &[coin(1_000_000, LENDING_DENOM)]),
        ExecuteMsg::Repay {},
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("No borrow to repay"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn repay_fails_zero_amount() {
    let (mut deps, env, _) = setup_borrower_with_debt();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[coin(0, LENDING_DENOM)]),
        ExecuteMsg::Repay {},
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("below minimum"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
