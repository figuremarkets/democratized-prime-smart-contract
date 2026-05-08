//! Tests for WithdrawReserve execute: success to contract owner or explicit recipient, assets-liabilities tie out,
//! and failures for non-owner, with funds, and when no accrued reserve.

use crate::constants::ATTRIBUTE_ACTION_NAME;
use crate::contract::execute;
use crate::execute::withdraw_reserve::{ACTION, ASSERT_OWNER_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{get_reserve_state_v1, set_reserve_state_v1};
use crate::tests::reserve_invariant::assert_assets_liabilities_tie_out_with_tolerance;
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use crate::utils::{scaled_to_underlying_borrow, scaled_to_underlying_liquidity};
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256,
    QuerierResult, SystemError, SystemResult, Timestamp, Uint128, WasmQuery,
};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use democratized_prime_lib::common::ContractError;
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use std::collections::HashMap;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const LENDING_DENOM: &str = "uylds.fcc";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
/// Valid bech32 address used as reserve recipient in tests (from transfer_tests).
const RECIPIENT: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";
/// Allow drift from scaled↔underlying and index updates before reserve send (see tests::reserve_invariant).
const TOLERANCE_BASE_UNITS: u128 = 10;

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
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128),
        min_lend: Uint128::new(1),
        min_borrow: Uint128::new(1),
        supported_collateral_assets: vec![CollateralAssetV1 {
            asset_id: "asset.one".to_string(),
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

/// Instantiate, lend, add collateral, borrow, advance time so accrued_reserve > 0.
fn setup_with_accrued_reserve() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
) {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut env = mock_env();

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert("asset.one".to_string(), price_entry("100"));
    set_oracle_prices(&mut deps.querier, prices);

    let msg = default_instantiate_msg();
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate");

    let lend_amount = 100_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked("tp1lender"),
            &[coin(lend_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    const BORROWER: &str = "tp1borrower";
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(200_000u128, "asset.one")],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(10_000_000),
        },
    )
    .expect("borrow");

    env.block.time = Timestamp::from_seconds(env.block.time.seconds() + 31_536_000);
    (deps, env)
}

#[test]
fn withdraw_reserve_succeeds_to_owner_when_recipient_none() {
    let (mut deps, env) = setup_with_accrued_reserve();
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

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .expect("withdraw_reserve should succeed");

    assert_eq!(res.messages.len(), 1);
    let amount = match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send {
            to_address,
            amount: coins,
        }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(coins.len(), 1);
            assert_eq!(coins[0].denom, LENDING_DENOM);
            coins[0].amount.u128()
        }
        _ => panic!("expected Bank Send"),
    };
    assert!(
        amount > 0,
        "accrued reserve should be positive after accrual"
    );
    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);

    let reserve_after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve_after.accrued_reserve, 0);
    let expected_implied_after = implied_before.saturating_sub(amount);
    assert_assets_liabilities_tie_out_with_tolerance(
        &reserve_after,
        "after withdraw_reserve to owner default recipient",
        Some(expected_implied_after),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

#[test]
fn withdraw_reserve_succeeds_to_recipient() {
    let (mut deps, env) = setup_with_accrued_reserve();
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

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve {
            recipient: Some(RECIPIENT.to_string()),
        },
    )
    .expect("withdraw_reserve should succeed");

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    let amount = match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send {
            to_address,
            amount: coins,
        }) => {
            assert_eq!(to_address.as_str(), RECIPIENT);
            coins[0].amount.u128()
        }
        _ => panic!("expected Bank Send"),
    };
    assert!(amount > 0);
    let reserve_after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve_after.accrued_reserve, 0);
    let expected_implied_after = implied_before.saturating_sub(amount);
    assert_assets_liabilities_tie_out_with_tolerance(
        &reserve_after,
        "after withdraw_reserve to recipient",
        Some(expected_implied_after),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

#[test]
fn withdraw_reserve_fails_non_owner() {
    let (mut deps, env) = setup_with_accrued_reserve();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked("tp1lender"), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn withdraw_reserve_fails_with_funds() {
    let (mut deps, env) = setup_with_accrued_reserve();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .unwrap_err();

    match &err {
        ContractError::InvalidFundsError { message } => {
            assert!(message.contains("No funds accepted"));
        }
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn withdraw_reserve_fails_when_no_accrued_reserve() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();

    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        default_instantiate_msg(),
    )
    .expect("instantiate");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(message.contains("No accrued reserve to withdraw"));
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

#[test]
fn withdraw_reserve_fails_when_deficit_positive() {
    let (mut deps, env) = setup_with_accrued_reserve();
    let mut reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    reserve.deficit_underlying = 1;
    set_reserve_state_v1(deps.as_mut().storage, &reserve).unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("deficit_underlying"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}
