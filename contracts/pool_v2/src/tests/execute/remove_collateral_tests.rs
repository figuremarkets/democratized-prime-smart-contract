//! Tests for RemoveCollateral execute: success (partial with debt, no debt, all when no debt);
//! failures for empty to_remove, zero amount, unsupported denom, insufficient collateral,
//! removal that would make the position unhealthy, zero lending-denom oracle price when debtor
//! checks apply, and with funds (no funds accepted).

use crate::constants::ATTRIBUTE_ACTION_NAME;
use crate::contract::execute;
use crate::execute::remove_collateral::ACTION;
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::get_borrower_collateral;
use crate::tests::fixtures::stale_oracle_price;
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256,
    QuerierResult, SystemError, SystemResult, Uint128, WasmQuery,
};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const BORROWER: &str = "tp1borrower";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
/// "u" prefix => 1 ylds.fcc = 10^6 uylds.fcc.
const LENDING_DENOM: &str = "uylds.fcc";
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

/// Borrower has 250 BTC collateral, no debt. Oracle set. Use for no-debt remove tests.
fn setup_collateral_no_debt() -> (
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

    (deps, env)
}

/// Borrower has 250 BTC collateral and 10M debt. LTV ≈ 10/14 ≈ 0.71. Margin 0.80.
/// Removing more than ~27 BTC would make LTV >= 0.80 (unhealthy).
fn setup_collateral_with_debt() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
) {
    let (mut deps, env) = setup_collateral_no_debt();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(10_000_000),
        },
    )
    .expect("borrow should succeed");
    (deps, env)
}

#[test]
fn remove_collateral_fails_with_funds() {
    let (mut deps, env) = setup_collateral_no_debt();
    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(100))]);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();

    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn remove_collateral_fails_when_oracle_price_is_stale() {
    let (mut deps, env) = setup_collateral_with_debt();
    let mut prices = HashMap::new();
    prices.insert(
        LENDING_DENOM.to_string(),
        stale_oracle_price(Decimal256::from_str("1.0").unwrap(), env.block.time),
    );
    prices.insert(
        COLLATERAL_BTC.to_string(),
        stale_oracle_price(Decimal256::from_str(BTC_PRICE_USD).unwrap(), env.block.time),
    );
    set_oracle_prices(&mut deps.querier, prices);

    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(10))]);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();

    match &err {
        ContractError::StalePriceDataError { .. } => {}
        _ => panic!("expected StalePriceDataError, got {:?}", err),
    }
}

#[test]
fn remove_collateral_succeeds_partial_with_debt() {
    let (mut deps, env) = setup_collateral_with_debt();
    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(20))]);

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral {
            to_remove: to_remove.clone(),
        },
    )
    .expect("remove_collateral should succeed");

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), BORROWER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, COLLATERAL_BTC);
            assert_eq!(amount[0].amount, Uint128::new(20));
        }
        _ => panic!("expected BankMsg::Send, got {:?}", res.messages[0].msg),
    }
    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].key, "borrower");
    assert_eq!(res.attributes[1].value, BORROWER);
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);

    let collateral = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(collateral.amounts.get(COLLATERAL_BTC), Some(&230));
}

#[test]
fn remove_collateral_succeeds_no_debt() {
    let (mut deps, env) = setup_collateral_no_debt();
    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(100))]);

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral {
            to_remove: to_remove.clone(),
        },
    )
    .expect("remove_collateral should succeed");

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), BORROWER);
            assert_eq!(amount[0].denom, COLLATERAL_BTC);
            assert_eq!(amount[0].amount, Uint128::new(100));
        }
        _ => panic!("expected BankMsg::Send"),
    }
    let collateral = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(collateral.amounts.get(COLLATERAL_BTC), Some(&150));
}

#[test]
fn remove_collateral_succeeds_all_when_no_debt() {
    let (mut deps, env) = setup_collateral_no_debt();
    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(250))]);

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .expect("remove_collateral should succeed");

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_eq!(res.messages.len(), 1);
    let collateral = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert!(collateral.amounts.is_empty());
}

#[test]
fn remove_collateral_fails_empty_to_remove() {
    let (mut deps, env) = setup_collateral_no_debt();
    let to_remove = BTreeMap::new();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("At least one collateral amount to remove must be specified"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn remove_collateral_fails_zero_amount() {
    let (mut deps, env) = setup_collateral_no_debt();
    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::zero())]);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("must be positive"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn remove_collateral_fails_unsupported_denom() {
    let (mut deps, env) = setup_collateral_no_debt();
    let to_remove = BTreeMap::from([("unsupported.denom".to_string(), Uint128::new(100))]);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Unsupported collateral asset"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn remove_collateral_fails_insufficient_collateral() {
    let (mut deps, env) = setup_collateral_no_debt();
    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(500))]);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Insufficient collateral"));
            assert!(message.contains("250"));
            assert!(message.contains("500"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn remove_collateral_fails_would_be_unhealthy() {
    let (mut deps, env) = setup_collateral_with_debt();
    // 250 - 30 = 220 BTC. Collateral value 220*70k*0.8 = 12.32M. LTV = 10/12.32 ≈ 0.81 > 0.80 margin.
    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(30))]);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
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
        _ => panic!("expected IllegalArgumentError (health), got {:?}", err),
    }
}

#[test]
fn remove_collateral_fails_remove_all_with_debt() {
    let (mut deps, env) = setup_collateral_with_debt();
    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(250))]);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("No collateral for loans"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn remove_collateral_fails_when_lending_denom_price_is_zero() {
    let (mut deps, env) = setup_collateral_with_debt();
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("0"));
    prices.insert(COLLATERAL_BTC.to_string(), price_entry(BTC_PRICE_USD));
    set_oracle_prices(&mut deps.querier, prices);

    let to_remove = BTreeMap::from([(COLLATERAL_BTC.to_string(), Uint128::new(1))]);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Lending denom price is zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
