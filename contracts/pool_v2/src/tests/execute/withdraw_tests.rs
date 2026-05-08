//! Tests for Withdraw execute: user path (Receive with Withdraw/WithdrawExact) and owner path
//! (ExecuteMsg::Withdraw). User: success exact and excess refunded, failures for zero amount,
//! amount exceeds cash, insufficient repo token sent, no funds, two coins, wrong denom.
//! WithdrawExact: success, zero amount, no funds, two coins, wrong denom, exceeds cash.
//! Owner (ExecuteMsg::Withdraw): full withdrawal, partial amount, reject non-owner, reject when lender has no balance,
//! reject when amount exceeds lender's supply, reject when amount exceeds cash, reject with funds.

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_LENDER, ATTRIBUTE_SCALED_AMOUNT,
};
use crate::contract::execute;
use crate::execute::withdraw::{ACTION, ACTION_EXACT, ASSERT_OWNER_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::execute::Cw20ReceivePayload;
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::get_reserve_state_v1;
use crate::tests::reserve_invariant::{
    assert_assets_liabilities_tie_out, assert_assets_liabilities_tie_out_with_tolerance,
};
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use crate::utils::{
    scaled_to_underlying_borrow, scaled_to_underlying_liquidity, underlying_to_scaled_liquidity,
};
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256, Env,
    MemoryStorage, OwnedDeps, QuerierResult, SystemError, SystemResult, Uint128, WasmMsg,
    WasmQuery,
};
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20ReceiveMsg};
use democratized_prime_lib::common::ContractError;
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use serde_json::{from_slice as json_from_slice, Value as JsonValue};
use std::collections::HashMap;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
/// Valid Provenance bech32 (so addr_validate passes in Receive handler).
const LENDER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";
const LENDING_DENOM: &str = "uylds.fcc";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
/// Valid Provenance bech32 for borrower in setup_with_liquidity_and_borrow (addr_validate would be used if we passed it as string).
const BORROWER: &str = "tp1w9p4tkctug2jyyx663f77x7e5cdry067z6xee4";

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

/// Instantiate and lend so there is liquidity to withdraw. Returns (deps, env, total_scaled_liquidity after supply).
fn setup_with_liquidity() -> (
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
    .expect("instantiate");

    let lend_amount = 100_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(LENDER),
            &[coin(lend_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    (deps, env, reserve.total_scaled_liquidity)
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

/// Like setup_with_liquidity but with oracle, add_collateral, and borrow so cash is reduced.
/// Lend 100M, borrow 60M, so cash = 40M. Use before mock_repo_scaled_balance for owner tests that need low cash.
fn setup_with_liquidity_and_borrow() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
) {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();

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
            &Addr::unchecked(LENDER),
            &[coin(lend_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    // Collateral: 1M asset.one at $100, 80% haircut => 80M; borrow 60M so LTV 75% < 80% (unhealthy). Cash = 40M.
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(1_000_000u128, "asset.one")],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(60_000_000),
        },
    )
    .expect("borrow");

    (deps, env)
}

/// Allow drift from scaled↔underlying floor/ceil and index truncation (see tests::reserve_invariant).
const TOLERANCE_BASE_UNITS: u128 = 10;

#[test]
fn withdraw_succeeds_and_burns_exact_scaled_sends_underlying() {
    let (mut deps, env, total_scaled) = setup_with_liquidity();
    let withdraw_underlying = 50_000_000u128;
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let liq_before =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)
            .unwrap();
    let bor_before =
        scaled_to_underlying_borrow(reserve.total_scaled_borrow, reserve.borrow_index).unwrap();
    let scaled_to_remove =
        underlying_to_scaled_liquidity(withdraw_underlying, reserve.liquidity_index)
            .expect("scaled");
    // Pool sends scaled_to_underlying(scaled), not the requested amount (avoids over-credit leak)
    let actual_sent =
        scaled_to_underlying_liquidity(scaled_to_remove, reserve.liquidity_index).unwrap();
    let expected_implied_after = liq_before
        .saturating_sub(bor_before)
        .saturating_sub(actual_sent)
        .saturating_add(reserve.accrued_reserve);

    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled_to_remove),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .expect("withdraw should succeed");

    // Burn + Send lending to user (no refund when exact)
    assert_eq!(res.messages.len(), 2, "burn + send underlying");
    match &res.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute { contract_addr, .. }) => {
            assert_eq!(
                contract_addr.as_str(),
                REPO_TOKEN_CW20,
                "expected CW20 burn"
            );
        }
        _ => panic!("expected Wasm Execute (burn)"),
    }
    match &res.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), LENDER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, LENDING_DENOM);
            assert_eq!(amount[0].amount.u128(), actual_sent);
        }
        _ => panic!("expected Bank Send"),
    }

    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].value, LENDER);
    assert_eq!(res.attributes[2].value, actual_sent.to_string());
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[3].value, scaled_to_remove.to_string());
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);

    let reserve_after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        reserve_after.total_scaled_liquidity,
        total_scaled - scaled_to_remove
    );
    assert_assets_liabilities_tie_out_with_tolerance(
        &reserve_after,
        "after withdraw (exact)",
        Some(expected_implied_after),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

#[test]
fn withdraw_refunds_excess_repo_token_sent() {
    let (mut deps, env, total_scaled) = setup_with_liquidity();
    let withdraw_underlying = 30_000_000u128;
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let liq_before =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)
            .unwrap();
    let bor_before =
        scaled_to_underlying_borrow(reserve.total_scaled_borrow, reserve.borrow_index).unwrap();
    let scaled_to_remove =
        underlying_to_scaled_liquidity(withdraw_underlying, reserve.liquidity_index)
            .expect("scaled");
    let actual_sent =
        scaled_to_underlying_liquidity(scaled_to_remove, reserve.liquidity_index).unwrap();
    let expected_implied_after = liq_before
        .saturating_sub(bor_before)
        .saturating_sub(actual_sent)
        .saturating_add(reserve.accrued_reserve);
    // Send more than needed (e.g. user sent full balance)
    let sent_scaled = scaled_to_remove + 10_000_000u128;

    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(sent_scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .expect("withdraw should succeed");

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    // Burn + Send underlying + Refund excess repo token (CW20 Transfer)
    assert_eq!(res.messages.len(), 3);
    match &res.messages[2].msg {
        CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) => {
            let bin = msg.as_slice();
            let _transfer: Cw20ExecuteMsg = from_json(bin).unwrap();
            // Refund amount is sent_scaled - scaled_to_remove
        }
        _ => panic!("expected refund CW20 Transfer"),
    }

    let reserve_after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        reserve_after.total_scaled_liquidity,
        total_scaled - scaled_to_remove
    );
    assert_assets_liabilities_tie_out_with_tolerance(
        &reserve_after,
        "after withdraw (refund)",
        Some(expected_implied_after),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

#[test]
fn withdraw_fails_zero_amount() {
    let (mut deps, env, _) = setup_with_liquidity();
    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let err = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(100u128),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::zero(),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Withdraw amount must be greater than zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_fails_amount_exceeds_cash() {
    let (mut deps, env, _) = setup_with_liquidity();
    // Supply was 100M; request more than that (no borrows, so cash = 100M)
    let withdraw_underlying = 150_000_000u128;
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let scaled = underlying_to_scaled_liquidity(withdraw_underlying, reserve.liquidity_index)
        .expect("scaled");

    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let err = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Insufficient liquidity"));
            assert!(message.contains("exceeds available cash"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_fails_insufficient_repo_token_sent() {
    let (mut deps, env, _) = setup_with_liquidity();
    let withdraw_underlying = 50_000_000u128;
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let scaled_needed =
        underlying_to_scaled_liquidity(withdraw_underlying, reserve.liquidity_index)
            .expect("scaled");
    // Send less than needed
    let sent = scaled_needed - 1;

    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let err = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(sent),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Insufficient repo token sent"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_fails_zero_received() {
    let (mut deps, env, _) = setup_with_liquidity();
    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let err = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::zero(),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(10_000_000),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Withdraw amount must be greater than zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_fails_wrong_cw20_sender() {
    let (mut deps, env, _) = setup_with_liquidity();
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let scaled = underlying_to_scaled_liquidity(50_000_000, reserve.liquidity_index).unwrap();
    let info = message_info(&Addr::unchecked("other_contract"), &[]);
    let err = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(50_000_000),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Only the repo token CW20"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

// --- require_commit_on_exit and commit_market_id tests ---

#[test]
fn withdraw_fails_when_require_commit_on_exit_and_commit_funds_not_true() {
    let (mut deps, env, _) = setup_with_liquidity();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(true),
        },
    )
    .expect("set require commit should succeed");

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let scaled = underlying_to_scaled_liquidity(5_000_000u128, reserve.liquidity_index).unwrap();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(5_000_000),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("commit_funds") && message.contains("commitment-on-exit"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_exact_fails_when_require_commit_on_exit_and_commit_funds_not_true() {
    let (mut deps, env, _) = setup_with_liquidity();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(true),
        },
    )
    .expect("set require commit should succeed");

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let scaled = underlying_to_scaled_liquidity(5_000_000u128, reserve.liquidity_index).unwrap();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled),
            msg: to_json_binary(&Cw20ReceivePayload::WithdrawExact { commit_funds: None }).unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("commit_funds") && message.contains("commitment-on-exit"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_succeeds_when_require_commit_on_exit_and_commit_funds_true() {
    let (mut deps, env, _) = setup_with_liquidity();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(true),
        },
    )
    .expect("set require commit should succeed");

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let withdraw_underlying = 5_000_000u128;
    let scaled =
        underlying_to_scaled_liquidity(withdraw_underlying, reserve.liquidity_index).unwrap();
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: Some(true),
            })
            .unwrap(),
        }),
    )
    .expect("withdraw with commit_funds true should succeed");

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_eq!(
        res.messages.len(),
        3,
        "burn + send + commit (commit_market_id set)"
    );
}

#[test]
fn withdraw_with_commit_funds_true_and_commit_market_id_emits_commit_message() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let mut msg = default_instantiate_msg();
    msg.commit_market_id = Some(1);
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate");

    let supply_amount = 100_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(LENDER),
            &[coin(supply_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("supply");

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let withdraw_underlying = 10_000_000u128;
    let scaled =
        underlying_to_scaled_liquidity(withdraw_underlying, reserve.liquidity_index).unwrap();
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: Some(true),
            })
            .unwrap(),
        }),
    )
    .expect("withdraw should succeed");

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    // Burn + Bank Send + commit message (Any/Stargate MsgExec/MsgCommitFundsRequest)
    assert_eq!(res.messages.len(), 3);
    let type_url = match &res.messages[2].msg {
        CosmosMsg::Any(any) => any.type_url.as_str(),
        other => panic!("expected third message to be commit (Any), got {:?}", other),
    };
    assert!(
        type_url.contains("MsgExec") || type_url.contains("authz"),
        "commit message should be MsgExec: got type_url {}",
        type_url
    );
}

#[test]
fn withdraw_with_commit_funds_true_and_no_commit_market_id_fails() {
    let (mut deps, env, _) = setup_with_liquidity();
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let withdraw_underlying = 10_000_000u128;
    let scaled =
        underlying_to_scaled_liquidity(withdraw_underlying, reserve.liquidity_index).unwrap();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: Some(true),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("commit_market_id is not configured"),
                "expected commit_market_id message, got {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

// --- WithdrawExact tests ---

#[test]
fn withdraw_exact_succeeds_burns_sent_repo_sends_underlying() {
    let (mut deps, env, total_scaled) = setup_with_liquidity();
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let liq_before =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)
            .unwrap();
    let bor_before =
        scaled_to_underlying_borrow(reserve.total_scaled_borrow, reserve.borrow_index).unwrap();
    // Withdraw half by sending that much scaled repo token (amount from funds only)
    let scaled_to_send = total_scaled / 2;
    let expected_underlying =
        scaled_to_underlying_liquidity(scaled_to_send, reserve.liquidity_index)
            .expect("underlying");
    let expected_implied_after = liq_before
        .saturating_sub(bor_before)
        .saturating_sub(expected_underlying)
        .saturating_add(reserve.accrued_reserve);

    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled_to_send),
            msg: to_json_binary(&Cw20ReceivePayload::WithdrawExact { commit_funds: None }).unwrap(),
        }),
    )
    .expect("withdraw_exact should succeed");

    assert_eq!(res.messages.len(), 2, "burn + send underlying");
    match &res.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), LENDER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, LENDING_DENOM);
            assert_eq!(amount[0].amount.u128(), expected_underlying);
        }
        _ => panic!("expected Bank Send"),
    }
    assert_eq!(res.attributes[0].value, ACTION_EXACT);
    assert_eq!(res.attributes[1].value, LENDER);
    assert_eq!(res.attributes[2].value, expected_underlying.to_string());
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[3].value, scaled_to_send.to_string());
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);

    let reserve_after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        reserve_after.total_scaled_liquidity,
        total_scaled - scaled_to_send
    );
    assert_assets_liabilities_tie_out_with_tolerance(
        &reserve_after,
        "after withdraw_exact",
        Some(expected_implied_after),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
}

#[test]
fn withdraw_exact_fails_zero_amount() {
    let (mut deps, env, _) = setup_with_liquidity();
    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let err = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::zero(),
            msg: to_json_binary(&Cw20ReceivePayload::WithdrawExact { commit_funds: None }).unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Withdraw amount must be greater than zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_exact_fails_wrong_cw20_sender() {
    let (mut deps, env, _) = setup_with_liquidity();
    let info = message_info(&Addr::unchecked("other_contract"), &[]);
    let err = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(10_000_000u128),
            msg: to_json_binary(&Cw20ReceivePayload::WithdrawExact { commit_funds: None }).unwrap(),
        }),
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Only the repo token CW20"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_exact_fails_exceeds_cash() {
    let (mut deps, env, _) = setup_with_liquidity();
    // Pool has 100M lent, no borrows → cash = 100M. Send more scaled than pool can cover:
    // e.g. 150M scaled → underlying would be > 100M cash.
    let scaled_over_cash = 150_000_000u128;

    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let err = execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled_over_cash),
            msg: to_json_binary(&Cw20ReceivePayload::WithdrawExact { commit_funds: None }).unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Insufficient liquidity"));
            assert!(message.contains("exceed available cash"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

// --- Assets / liabilities invariant (shared helper from reserve_invariant) ---

#[test]
fn withdraw_preserves_assets_liabilities_invariant() {
    let (mut deps, env, total_scaled) = setup_with_liquidity();
    let reserve_before = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let (liq_before, bor_before, _accrued_before) =
        assert_assets_liabilities_tie_out(&reserve_before, "before withdraw").unwrap();
    let cash_before = liq_before.saturating_sub(bor_before);

    let withdraw_underlying = 50_000_000u128;
    let scaled_to_remove =
        underlying_to_scaled_liquidity(withdraw_underlying, reserve_before.liquidity_index)
            .expect("scaled");
    let amount_sent =
        scaled_to_underlying_liquidity(scaled_to_remove, reserve_before.liquidity_index).unwrap();
    let info = message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER.to_string(),
            amount: Uint128::from(scaled_to_remove),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .expect("withdraw should succeed");
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    let reserve_after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    match &res.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send { amount, .. }) => {
            assert_eq!(amount[0].amount.u128(), amount_sent);
        }
        _ => panic!("expected Bank Send"),
    }

    let expected_implied_cash = cash_before
        .saturating_sub(amount_sent)
        .saturating_add(_accrued_before);
    let (liq_after, bor_after, accrued_after) = assert_assets_liabilities_tie_out_with_tolerance(
        &reserve_after,
        "after withdraw",
        Some(expected_implied_cash),
        TOLERANCE_BASE_UNITS,
    )
    .unwrap();
    let cash_after = liq_after.saturating_sub(bor_after);

    // Conservation: amount sent + remaining cash = cash before (reserve unchanged by withdraw)
    assert_eq!(
        amount_sent.saturating_add(cash_after),
        cash_before,
        "withdraw: amount_sent + cash_after must equal cash_before"
    );
    assert_eq!(bor_after, bor_before, "total_borrow unchanged by withdraw");
    assert_eq!(
        accrued_after, _accrued_before,
        "accrued_reserve unchanged by withdraw"
    );
    assert_eq!(
        reserve_after.total_scaled_liquidity,
        total_scaled - scaled_to_remove
    );
}

// --- Owner withdraw on behalf of lender (ExecuteMsg::Withdraw): mock repo ScaledBalance, then test success and failures ---

/// Mock repo token ScaledBalance query for REPO_TOKEN_CW20; returns BalanceResponse { balance }.
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
fn withdraw_owner_fails_with_funds() {
    let (mut deps, env, _) = setup_with_liquidity();
    mock_repo_scaled_balance(&mut deps.querier, 100_000_000);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: None,
            commit_funds: None,
        },
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
fn withdraw_owner_succeeds_full_withdrawal() {
    let (mut deps, env, _) = setup_with_liquidity();
    let scaled = 100_000_000u128;
    mock_repo_scaled_balance(&mut deps.querier, scaled);

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
    .expect("Withdraw (owner) should succeed");

    assert_eq!(res.messages.len(), 2);
    match &res.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr, msg, ..
        }) => {
            assert_eq!(contract_addr.as_str(), REPO_TOKEN_CW20);
            let exec: JsonValue = from_json(msg.as_slice()).unwrap();
            assert_eq!(
                exec.get("burn_from")
                    .and_then(|b| b.get("owner"))
                    .and_then(|o| o.as_str()),
                Some(LENDER)
            );
            assert_eq!(
                exec.get("burn_from")
                    .and_then(|b| b.get("amount"))
                    .and_then(|a| a.as_str()),
                Some("100000000")
            );
        }
        _ => panic!("expected first message to be Wasm BurnFrom"),
    }
    match &res.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send {
            to_address,
            amount: coins,
        }) => {
            assert_eq!(to_address.as_str(), LENDER);
            assert_eq!(coins.len(), 1);
            assert_eq!(coins[0].denom, LENDING_DENOM);
            assert_eq!(coins[0].amount.u128(), 100_000_000);
        }
        _ => panic!("expected second message to be Bank Send"),
    }
    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].key, ATTRIBUTE_LENDER);
    assert_eq!(res.attributes[1].value, LENDER);
    assert_eq!(res.attributes[2].key, ATTRIBUTE_AMOUNT);
    assert_eq!(res.attributes[2].value, "100000000");
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[3].value, "100000000");
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
}

#[test]
fn withdraw_owner_rejected_when_not_owner() {
    let (mut deps, env, _) = setup_with_liquidity();
    mock_repo_scaled_balance(&mut deps.querier, 100_000_000);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(LENDER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: None,
            commit_funds: None,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn withdraw_owner_rejected_when_lender_has_no_balance() {
    let (mut deps, env, _) = setup_with_liquidity();
    mock_repo_scaled_balance(&mut deps.querier, 0);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: None,
            commit_funds: None,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.to_lowercase().contains("no repo token balance"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_owner_amount_exceeds_lender_supply() {
    let (mut deps, env, _) = setup_with_liquidity();
    mock_repo_scaled_balance(&mut deps.querier, 100_000_000); // lender has 100M underlying

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: Some(Uint128::new(150_000_000)), // more than lender's supply
            commit_funds: None,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.to_lowercase().contains("exceeds lender's supply"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_owner_amount_exceeds_cash() {
    let (mut deps, env) = setup_with_liquidity_and_borrow(); // 100M lent, 60M borrowed => cash 40M
    mock_repo_scaled_balance(&mut deps.querier, 100_000_000); // lender has 100M supply

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: Some(Uint128::new(50_000_000)), // 50M > cash (40M)
            commit_funds: None,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.to_lowercase().contains("insufficient liquidity"));
            assert!(message.to_lowercase().contains("exceed available cash"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_owner_partial_amount_succeeds() {
    let (mut deps, env, _) = setup_with_liquidity();
    mock_repo_scaled_balance(&mut deps.querier, 100_000_000); // lender has 100M scaled

    let partial = 50_000_000u128;
    let reserve_before = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let expected_scaled_burn =
        underlying_to_scaled_liquidity(partial, reserve_before.liquidity_index).unwrap();
    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: Some(Uint128::new(partial)),
            commit_funds: None,
        },
    )
    .expect("Withdraw (owner) partial should succeed");

    assert_eq!(res.messages.len(), 2);
    match &res.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send {
            to_address,
            amount: coins,
        }) => {
            assert_eq!(to_address.as_str(), LENDER);
            assert_eq!(coins.len(), 1);
            assert_eq!(coins[0].denom, LENDING_DENOM);
            assert_eq!(coins[0].amount.u128(), partial);
        }
        _ => panic!("expected second message to be Bank Send"),
    }
    assert_eq!(res.attributes[2].key, ATTRIBUTE_AMOUNT);
    assert_eq!(res.attributes[2].value, partial.to_string());
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[3].value, expected_scaled_burn.to_string());
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
}

#[test]
fn withdraw_owner_with_commit_funds_true_and_no_commit_market_id_fails() {
    let (mut deps, env, _) = setup_with_liquidity();
    mock_repo_scaled_balance(&mut deps.querier, 10_000_000u128);
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: Some(Uint128::new(5_000_000)),
            commit_funds: Some(true),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("commit_market_id is not configured"),
                "expected commit_market_id message, got {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn withdraw_owner_with_commit_funds_true_and_commit_market_id_emits_commit_message() {
    let (mut deps, env, _) = setup_with_liquidity();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id");

    mock_repo_scaled_balance(&mut deps.querier, 50_000_000u128);
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: Some(Uint128::new(5_000_000)),
            commit_funds: Some(true),
        },
    )
    .expect("owner withdraw with commit_funds should succeed");

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_eq!(
        res.messages.len(),
        3,
        "burn + send + commit when commit_funds true and commit_market_id set"
    );
    let type_url = match &res.messages[2].msg {
        CosmosMsg::Any(any) => any.type_url.as_str(),
        other => panic!("expected third message to be commit (Any), got {:?}", other),
    };
    assert!(
        type_url.contains("MsgExec") || type_url.contains("authz"),
        "commit message should be MsgExec: got type_url {}",
        type_url
    );
}
