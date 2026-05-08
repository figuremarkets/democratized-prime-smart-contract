//! Tests for Borrow execute: success with collateral and healthy LTV; failures for zero amount,
//! below min_borrow, insufficient liquidity, no collateral, missing borrower attr, unhealthy LTV,
//! and with funds (no funds accepted).

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_SCALED_AMOUNT};
use crate::contract::execute;
use crate::execute::borrow::ACTION;
use crate::instantiate::instantiate_contract;
use crate::model::{BorrowerCollateralV1, CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{get_reserve_state_v1, get_scaled_borrow, set_borrower_collateral};
use crate::tests::fixtures::{fresh_oracle_price, stale_oracle_price};
use crate::tests::reserve_invariant::assert_reserve_assets_liabilities_tie_out_with_tolerance;
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use crate::utils::scaled_to_underlying_liquidity;
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256, Env,
    MemoryStorage, OwnedDeps, QuerierResult, SystemError, SystemResult, Uint128, WasmQuery,
};
use democratized_prime_lib::common::ContractError;
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use provwasm_std::types::provenance::attribute::v1::{
    QueryAttributeRequest, QueryAttributeResponse,
};
use std::collections::HashMap;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const BORROWER: &str = "tp1borrower";
const LENDING_DENOM: &str = "uylds.fcc";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
/// Bitcoin collateral (e.g. wrapped nBTC); oracle price $70k in tests.
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

/// Mock oracle to return the given prices for GetPricesByAsset.
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

/// Instantiate, lend, add collateral for borrower, and mock oracle so LTV is healthy for borrowing.
fn setup_borrow_ready() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
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

    // Lend so there is cash
    let lend_amount = 100_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(lend_amount, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend should succeed");

    // Add collateral: 250 BTC at $70k with 80% haircut -> 250 * 70000 * 0.8 = $14M collateral value; borrow 10M -> LTV ≈ 0.71 < 0.80
    let btc_collateral_amount = 250u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(btc_collateral_amount, COLLATERAL_BTC)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral should succeed");

    // Oracle: lending denom $1, BTC $70k
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_BTC.to_string(), price_entry(BTC_PRICE_USD));
    set_oracle_prices(&mut deps.querier, prices);

    (deps, env)
}

/// Allow drift from scaled↔underlying floor/ceil and index truncation (see tests::reserve_invariant).
const TOLERANCE_BASE_UNITS: u128 = 10;

#[test]
fn borrow_succeeds_and_updates_state() {
    let (mut deps, env) = setup_borrow_ready();
    let amount = Uint128::new(10_000_000);
    let lend_amount = 100_000_000u128;
    let expected_implied_cash = lend_amount.saturating_sub(amount.u128());

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow { amount },
    )
    .expect("borrow should succeed");

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send {
            to_address,
            amount: coins,
        }) => {
            assert_eq!(to_address.as_str(), BORROWER);
            assert_eq!(coins.len(), 1);
            assert_eq!(coins[0].denom, LENDING_DENOM);
            assert_eq!(coins[0].amount, amount);
        }
        _ => panic!("expected Bank::Send message, got {:?}", res.messages[0].msg),
    }
    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].key, "borrower");
    assert_eq!(res.attributes[1].value, BORROWER);
    assert_eq!(res.attributes[2].key, "amount");
    assert_eq!(res.attributes[2].value, amount.to_string());
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    // scaled_amount is the transaction delta (like lend/repay/withdraw), not total scaled debt.
    let scaled_after = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(res.attributes[3].value, scaled_after.to_string());
    assert!(scaled_after > 0);
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_reserve_assets_liabilities_tie_out_with_tolerance(
        deps.as_ref().storage,
        "after borrow",
        Some(expected_implied_cash),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

#[test]
fn borrow_second_time_emits_scaled_delta_not_total_scaled_debt() {
    let (mut deps, env) = setup_borrow_ready();
    let first = Uint128::new(5_000_000);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow { amount: first },
    )
    .expect("first borrow should succeed");

    let scaled_after_first = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    let second = Uint128::new(3_000_000);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow { amount: second },
    )
    .expect("second borrow should succeed");

    let scaled_after_second = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    let scaled_delta = scaled_after_second - scaled_after_first;
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[3].value, scaled_delta.to_string());
    assert_ne!(
        res.attributes[3].value,
        scaled_after_second.to_string(),
        "attribute must be delta only, not total scaled debt"
    );
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
}

#[test]
fn borrow_fails_with_funds() {
    let (mut deps, env) = setup_borrow_ready();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(10_000_000),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn borrow_fails_zero_amount() {
    let (mut deps, env) = setup_borrow_ready();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::zero(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Borrow amount must be positive"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn borrow_fails_below_min_borrow() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let mut msg = default_instantiate_msg();
    msg.min_borrow = Uint128::new(1000);
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap();

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(100_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(250, COLLATERAL_BTC)]),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_BTC.to_string(), price_entry(BTC_PRICE_USD));
    set_oracle_prices(&mut deps.querier, prices);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(500),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("min_borrow"));
            assert!(message.contains("1000"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn borrow_fails_insufficient_liquidity() {
    let (mut deps, env) = setup_borrow_ready();
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let total_liquidity =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)
            .unwrap();
    let too_much = total_liquidity + 1;

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(too_much),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Insufficient liquidity"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn borrow_fails_no_collateral() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        default_instantiate_msg(),
    )
    .unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(100_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .unwrap();
    // Do not add collateral for BORROWER

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(10_000_000),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Cannot borrow without collateral"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn borrow_fails_borrower_attr_required_but_missing() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();

    // Mock empty attributes once; we never call add_collateral so we never need attr present.
    let empty_attr = QueryAttributeResponse {
        account: BORROWER.to_string(),
        attributes: vec![],
        pagination: None,
    };
    QueryAttributeRequest::mock_response(&mut deps.querier, empty_attr);

    let mut msg = default_instantiate_msg();
    msg.borrower_required_attrs = vec!["borrower.kyc".to_string()];
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(100_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .unwrap();

    // Give borrower collateral in storage so borrow has collateral but fails on attr check.
    let mut collateral = BorrowerCollateralV1::default();
    collateral.amounts.insert(COLLATERAL_BTC.to_string(), 250);
    set_borrower_collateral(deps.as_mut().storage, BORROWER, &collateral).unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(10_000_000),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::NotAuthorizedError { message } => {
            assert!(message.contains("borrower"));
        }
        _ => panic!("expected NotAuthorizedError, got {:?}", err),
    }
}

#[test]
fn borrow_fails_ltv_unhealthy() {
    let (mut deps, env) = setup_borrow_ready();
    // Override oracle: BTC at $30k (e.g. downturn). Collateral value = 250 * 30000 * 0.8 = $6M.
    // Borrowing $5M -> LTV = 5/6 ≈ 0.833 > 0.80 margin, so borrow is rejected.
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_BTC.to_string(), price_entry("30000"));
    set_oracle_prices(&mut deps.querier, prices);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(5_000_000),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("Loan-to-value") || message.contains("threshold"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError (LTV), got {:?}", err),
    }
}

#[test]
fn borrow_fails_when_lending_denom_price_zero() {
    let (mut deps, env) = setup_borrow_ready();
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("0"));
    prices.insert(COLLATERAL_BTC.to_string(), price_entry(BTC_PRICE_USD));
    set_oracle_prices(&mut deps.querier, prices);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(10_000_000),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Lending denom price is zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn borrow_fails_when_oracle_price_is_stale_for_lending_denom() {
    let (mut deps, env) = setup_borrow_ready();
    let mut prices = HashMap::new();
    prices.insert(
        LENDING_DENOM.to_string(),
        stale_oracle_price(Decimal256::from_str("1.0").unwrap(), env.block.time),
    );
    prices.insert(
        COLLATERAL_BTC.to_string(),
        fresh_oracle_price(Decimal256::from_str(BTC_PRICE_USD).unwrap(), env.block.time),
    );
    set_oracle_prices(&mut deps.querier, prices);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(10_000_000),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::StalePriceDataError { .. } => {}
        _ => panic!("expected StalePriceDataError, got {:?}", err),
    }
}

#[test]
fn borrow_fails_when_oracle_price_is_stale_for_collateral_asset() {
    let (mut deps, env) = setup_borrow_ready();
    let mut prices = HashMap::new();
    prices.insert(
        LENDING_DENOM.to_string(),
        fresh_oracle_price(Decimal256::from_str("1.0").unwrap(), env.block.time),
    );
    prices.insert(
        COLLATERAL_BTC.to_string(),
        stale_oracle_price(Decimal256::from_str(BTC_PRICE_USD).unwrap(), env.block.time),
    );
    set_oracle_prices(&mut deps.querier, prices);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(10_000_000),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::StalePriceDataError { .. } => {}
        _ => panic!("expected StalePriceDataError, got {:?}", err),
    }
}
