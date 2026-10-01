//! Tests for WithdrawReserve execute: success to contract owner or explicit recipient, assets-liabilities tie out,
//! and failures for non-owner, with funds, and when no accrued reserve.

use crate::constants::{
    ATTRIBUTE_ACCRUED_RESERVE_REMAINING, ATTRIBUTE_ACTION_NAME, ATTRIBUTE_UNBACKED_RESERVE_WRITEOFF,
};
use crate::contract::execute;
use crate::execute::withdraw_reserve::{ACTION, ASSERT_OWNER_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::execute::Cw20ReceivePayload;
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{
    get_contract_state_v1, get_reserve_state_v1, get_scaled_borrow, set_reserve_state_v1,
};
use crate::tests::query::common::{CUSTODIAN, OWNER};
use crate::tests::reserve_invariant::assert_assets_liabilities_tie_out_with_tolerance;
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use crate::utils::{
    compute_effective_reserve, scaled_to_underlying_borrow, scaled_to_underlying_borrow_ceil,
    scaled_to_underlying_liquidity,
};
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256,
    QuerierResult, SystemError, SystemResult, Timestamp, Uint128, WasmQuery,
};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use cw20::{BalanceResponse, Cw20ReceiveMsg};
use democratized_prime_lib::common::ContractError;
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use serde_json::{from_slice as json_from_slice, Value as JsonValue};
use std::collections::HashMap;
use std::str::FromStr;

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
            fee_model: Default::default(),
            flat_fee_apr: Decimal256::zero(),
            seconds_per_year: 31_536_000,
        },
        lender_required_attrs: vec![],
        borrower_required_attrs: vec![],
        price_oracle_address: ORACLE.to_string(),
        max_borrower_collateral_types: 5,
        max_liquidation_staleness_seconds: 3600,
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
        custodian: CUSTODIAN.to_owned(),
        liquidation_access: Default::default(),
    }
}

fn price_entry(price: &str) -> AssetPriceResponseV1 {
    AssetPriceResponseV1::new(Decimal256::from_str(price).unwrap(), 0, u64::MAX)
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
                    Ok(PriceOracleQueryMsg::GetPricesByAsset { .. }) => {
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
    // The mock executor does not apply bank sends/receives. Model the contract's actual cash
    // after the 100m lend and 10m borrow so WithdrawReserve can perform its solvency query.
    deps.querier.mock_querier.bank.update_balance(
        env.contract.address.as_str(),
        vec![coin(lend_amount - 10_000_000, LENDING_DENOM)],
    );
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
fn withdraw_reserve_for_custodian_fails() {
    let (mut deps, env) = setup_with_accrued_reserve();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
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

#[test]
fn withdraw_reserve_caps_payout_at_bank_surplus_over_lender_claims() {
    let (mut deps, env) = setup_with_accrued_reserve();
    let mut reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    reserve.last_updated_at = env.block.time;
    let total_liquidity =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)
            .unwrap();
    let total_borrow =
        scaled_to_underlying_borrow(reserve.total_scaled_borrow, reserve.borrow_index).unwrap();
    let lender_claims = total_liquidity.saturating_sub(total_borrow);
    let bank_surplus = 1_000u128;
    reserve.accrued_reserve = 50_000_000;
    set_reserve_state_v1(deps.as_mut().storage, &reserve).unwrap();
    deps.querier.mock_querier.bank.update_balance(
        env.contract.address.as_str(),
        vec![coin(lender_claims + bank_surplus, LENDING_DENOM)],
    );

    let response = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .expect("solvent portion should be withdrawable");

    let amount = match &response.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { amount, .. }) => amount[0].amount.u128(),
        _ => panic!("expected Bank Send"),
    };
    assert_eq!(amount, bank_surplus);
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .accrued_reserve,
        0,
        "the unbacked portion must not remain claimable against future lender cash"
    );
    let writeoff = response
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_UNBACKED_RESERVE_WRITEOFF)
        .expect("capped withdraw must surface the unbacked writeoff");
    assert_eq!(writeoff.value, (50_000_000u128 - bank_surplus).to_string());
}

/// Worked example: scaled liquidity 1000 at index 1.08 (L = 1080), scaled borrow 1000 at index 1.09 (B = 1090).
const SIGNED_CAP_SCALED: u128 = 1_000;
const SIGNED_CAP_LIQUIDITY_INDEX: &str = "1.08";
const SIGNED_CAP_BORROW_INDEX: &str = "1.09";

fn sent_lending_amount(response: &cosmwasm_std::Response) -> u128 {
    match &response.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { amount, .. }) => {
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, LENDING_DENOM);
            amount[0].amount.u128()
        }
        _ => panic!("expected Bank Send"),
    }
}

fn writeoff_attribute(response: &cosmwasm_std::Response) -> Option<&str> {
    response
        .attributes
        .iter()
        .find(|attr| attr.key == ATTRIBUTE_UNBACKED_RESERVE_WRITEOFF)
        .map(|attr| attr.value.as_str())
}

struct ReserveSnapshot {
    total_scaled_liquidity: u128,
    liquidity_index: &'static str,
    total_scaled_borrow: u128,
    borrow_index: &'static str,
    accrued_reserve: u128,
    bank: u128,
}

/// Overwrite reserve totals and the lending-denom bank balance. `last_updated_at` matches the
/// block so `update_reserve_indexes` does not accrue before the payout math runs.
fn install_reserve_snapshot(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    env: &Env,
    snapshot: ReserveSnapshot,
) {
    let mut reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    reserve.last_updated_at = env.block.time;
    reserve.total_scaled_liquidity = snapshot.total_scaled_liquidity;
    reserve.liquidity_index = Decimal256::from_str(snapshot.liquidity_index).unwrap();
    reserve.total_scaled_borrow = snapshot.total_scaled_borrow;
    reserve.borrow_index = Decimal256::from_str(snapshot.borrow_index).unwrap();
    reserve.accrued_reserve = snapshot.accrued_reserve;
    reserve.deficit_underlying = 0;
    set_reserve_state_v1(deps.as_mut().storage, &reserve).unwrap();
    deps.querier.mock_querier.bank.update_balance(
        env.contract.address.as_str(),
        vec![coin(snapshot.bank, LENDING_DENOM)],
    );
}

#[test]
fn withdraw_reserve_keeps_backed_fees_booked_when_borrow_exceeds_liquidity() {
    let (mut deps, env) = setup_with_accrued_reserve();
    // L = 1080, B = 1090, AR = 20, bank = 10 → pay 10, AR stays 10, no writeoff.
    install_reserve_snapshot(
        &mut deps,
        &env,
        ReserveSnapshot {
            total_scaled_liquidity: SIGNED_CAP_SCALED,
            liquidity_index: SIGNED_CAP_LIQUIDITY_INDEX,
            total_scaled_borrow: SIGNED_CAP_SCALED,
            borrow_index: SIGNED_CAP_BORROW_INDEX,
            accrued_reserve: 20,
            bank: 10,
        },
    );
    let before = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        scaled_to_underlying_liquidity(before.total_scaled_liquidity, before.liquidity_index)
            .unwrap(),
        1_080
    );
    assert_eq!(
        scaled_to_underlying_borrow(before.total_scaled_borrow, before.borrow_index).unwrap(),
        1_090
    );

    let response = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .expect("backed surplus should be withdrawable");

    let first_pay = sent_lending_amount(&response);
    assert_eq!(first_pay, 10);
    assert!(
        writeoff_attribute(&response).is_none(),
        "fully backed remainder must not be written off"
    );
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .accrued_reserve,
        10
    );
    assert_eq!(
        response
            .attributes
            .iter()
            .find(|a| a.key == ATTRIBUTE_ACCRUED_RESERVE_REMAINING)
            .map(|a| a.value.as_str()),
        Some("10")
    );

    // Borrowers repay in full. Bank then holds the repaid principal plus the fee still booked.
    let mut reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    reserve.total_scaled_borrow = 0;
    reserve.last_updated_at = env.block.time;
    set_reserve_state_v1(deps.as_mut().storage, &reserve).unwrap();
    let bank_after_repay = 1_090u128;
    deps.querier.mock_querier.bank.update_balance(
        env.contract.address.as_str(),
        vec![coin(bank_after_repay, LENDING_DENOM)],
    );

    let follow_up = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .expect("repaid fees should be withdrawable");

    let second_pay = sent_lending_amount(&follow_up);
    assert_eq!(second_pay, 10);
    assert!(writeoff_attribute(&follow_up).is_none());
    let after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(after.accrued_reserve, 0);
    let lender_claims =
        scaled_to_underlying_liquidity(after.total_scaled_liquidity, after.liquidity_index)
            .unwrap();
    let bank_received = 10u128 + bank_after_repay;
    assert_eq!(
        first_pay + second_pay + lender_claims,
        bank_received,
        "coins sent plus remaining lender claims must equal every coin the bank received"
    );
}

#[test]
fn withdraw_reserve_writes_off_only_unbacked_when_liquidity_covers_claims() {
    let (mut deps, env) = setup_with_accrued_reserve();
    // L = 1000, B = 0, AR = 50, bank = 1005 → pay 5, AR = 0, writeoff = 45.
    install_reserve_snapshot(
        &mut deps,
        &env,
        ReserveSnapshot {
            total_scaled_liquidity: 1_000,
            liquidity_index: "1",
            total_scaled_borrow: 0,
            borrow_index: "1",
            accrued_reserve: 50,
            bank: 1_005,
        },
    );

    let response = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .expect("solvent surplus should be withdrawable");

    assert_eq!(sent_lending_amount(&response), 5);
    assert_eq!(writeoff_attribute(&response), Some("45"));
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .accrued_reserve,
        0
    );
}

#[test]
fn withdraw_reserve_writes_off_only_the_unbacked_slice_when_borrow_exceeds_liquidity() {
    let (mut deps, env) = setup_with_accrued_reserve();
    // L = 1080, B = 1090, AR = 25, bank = 10 → pay 10, AR = 10, writeoff = 5.
    install_reserve_snapshot(
        &mut deps,
        &env,
        ReserveSnapshot {
            total_scaled_liquidity: SIGNED_CAP_SCALED,
            liquidity_index: SIGNED_CAP_LIQUIDITY_INDEX,
            total_scaled_borrow: SIGNED_CAP_SCALED,
            borrow_index: SIGNED_CAP_BORROW_INDEX,
            accrued_reserve: 25,
            bank: 10,
        },
    );

    let response = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .expect("backed portion should be withdrawable");

    assert_eq!(sent_lending_amount(&response), 10);
    assert_eq!(writeoff_attribute(&response), Some("5"));
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .accrued_reserve,
        10
    );
}

#[test]
fn withdraw_reserve_rejects_when_nothing_is_payable_and_leaves_accrued_reserve() {
    let (mut deps, env) = setup_with_accrued_reserve();
    // bank = 0, B > L, AR > 0 → error and no state change.
    install_reserve_snapshot(
        &mut deps,
        &env,
        ReserveSnapshot {
            total_scaled_liquidity: SIGNED_CAP_SCALED,
            liquidity_index: SIGNED_CAP_LIQUIDITY_INDEX,
            total_scaled_borrow: SIGNED_CAP_SCALED,
            borrow_index: SIGNED_CAP_BORROW_INDEX,
            accrued_reserve: 20,
            bank: 0,
        },
    );
    let before = get_reserve_state_v1(deps.as_ref().storage).unwrap();

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
                message.contains("No solvent accrued reserve available to withdraw"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
    let after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(after.accrued_reserve, before.accrued_reserve);
    assert_eq!(after, before);
}

const E2E_LENDER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";
const E2E_BORROWER: &str = "tp1w9p4tkctug2jyyx663f77x7e5cdry067z6xee4";
const E2E_COLLATERAL: &str = "asset.one";
const BANK_DUST_TOLERANCE: u128 = 10;

fn sync_contract_bank(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    env: &Env,
    balance: u128,
) {
    deps.querier.mock_querier.bank.update_balance(
        env.contract.address.as_str(),
        vec![coin(balance, LENDING_DENOM)],
    );
}

fn bank_send_amount(response: &cosmwasm_std::Response) -> u128 {
    match &response.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { amount, .. }) => amount[0].amount.u128(),
        _ => panic!("expected Bank Send"),
    }
}

fn response_attribute<'a>(response: &'a cosmwasm_std::Response, key: &str) -> Option<&'a str> {
    response
        .attributes
        .iter()
        .find(|a| a.key == key)
        .map(|a| a.value.as_str())
}

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

fn advance_until_borrow_exceeds_liquidity(
    deps: &OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    env: &mut Env,
    min_accrued_reserve: u128,
) -> u128 {
    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let start = env.block.time.seconds();
    let accrued_at_jump;
    let mut t = start;
    loop {
        t += 86_400;
        let ts = Timestamp::from_seconds(t);
        let effective = compute_effective_reserve(deps.as_ref().storage, ts, &contract.rate_params)
            .expect("accrual projection");
        let total_liquidity = scaled_to_underlying_liquidity(
            effective.total_scaled_liquidity,
            effective.liquidity_index,
        )
        .unwrap();
        let total_borrow =
            scaled_to_underlying_borrow(effective.total_scaled_borrow, effective.borrow_index)
                .unwrap();
        let borrow_surplus = total_borrow.saturating_sub(total_liquidity);
        if borrow_surplus > 500_000 && effective.accrued_reserve >= min_accrued_reserve {
            env.block.time = ts;
            accrued_at_jump = effective.accrued_reserve;
            break;
        }
        assert!(
            t <= start + 200 * 31_536_000,
            "borrow did not exceed liquidity within 200 years of simulation"
        );
    }
    accrued_at_jump
}

/// End-to-end signed-cap reserve path without hand-built reserve snapshots.
#[test]
fn withdraw_reserve_e2e_accrual_partial_payout_repay_and_lender_exit() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut env = mock_env();

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(E2E_COLLATERAL.to_string(), price_entry("100"));
    set_oracle_prices(&mut deps.querier, prices);

    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        default_instantiate_msg(),
    )
    .expect("instantiate");

    let lend_amount = 100_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(E2E_LENDER),
            &[coin(lend_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("lend");
    let lender_scaled = get_reserve_state_v1(deps.as_ref().storage)
        .unwrap()
        .total_scaled_liquidity;

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(E2E_BORROWER),
            &[coin(2_000_000, E2E_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add collateral");

    let borrow_amount = 99_900_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(E2E_BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(borrow_amount),
        },
    )
    .expect("borrow");

    let mut contract_bank = lend_amount - borrow_amount;
    sync_contract_bank(&mut deps, &env, contract_bank);

    let mut higher_rf = default_instantiate_msg().rate_params;
    higher_rf.reserve_factor = Decimal256::from_str("0.05").unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: higher_rf,
        },
    )
    .expect("raise reserve factor for meaningful fee accrual");

    let min_accrued = contract_bank * 3;
    let accrued_from_accrual = advance_until_borrow_exceeds_liquidity(&deps, &mut env, min_accrued);
    assert!(accrued_from_accrual > contract_bank);

    let small_repay = 10_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(E2E_BORROWER),
            &[coin(small_repay, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .expect("partial repay");
    contract_bank += small_repay;
    sync_contract_bank(&mut deps, &env, contract_bank);

    let mut owner_paid = 0u128;
    let first = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .expect("first withdraw_reserve");
    let first_pay = bank_send_amount(&first);
    owner_paid += first_pay;
    contract_bank -= first_pay;
    sync_contract_bank(&mut deps, &env, contract_bank);

    let reserve_after_first = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert!(reserve_after_first.accrued_reserve > 0);
    assert!(response_attribute(&first, ATTRIBUTE_UNBACKED_RESERVE_WRITEOFF).is_none());
    assert_eq!(
        response_attribute(&first, ATTRIBUTE_ACCRUED_RESERVE_REMAINING),
        Some(reserve_after_first.accrued_reserve.to_string().as_str())
    );

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let scaled = get_scaled_borrow(deps.as_ref().storage, E2E_BORROWER).unwrap();
    let full_repay = scaled_to_underlying_borrow_ceil(scaled, reserve.borrow_index).unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(E2E_BORROWER),
            &[coin(full_repay, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .expect("full repay");
    contract_bank += full_repay;
    sync_contract_bank(&mut deps, &env, contract_bank);

    let second = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .expect("second withdraw_reserve");
    let second_pay = bank_send_amount(&second);
    owner_paid += second_pay;
    contract_bank -= second_pay;
    sync_contract_bank(&mut deps, &env, contract_bank);
    assert!(response_attribute(&second, ATTRIBUTE_ACCRUED_RESERVE_REMAINING).is_none());
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .accrued_reserve,
        0
    );

    mock_repo_scaled_balance(&mut deps.querier, lender_scaled);
    let withdraw = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: E2E_LENDER.to_string(),
            amount: Uint128::from(lender_scaled),
            msg: to_json_binary(&Cw20ReceivePayload::WithdrawExact { commit_funds: None }).unwrap(),
        }),
    )
    .expect("lender withdraw_exact");
    let lender_sent = match &withdraw.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send { amount, .. }) => amount[0].amount.u128(),
        _ => panic!("expected lender bank send"),
    };
    contract_bank -= lender_sent;
    sync_contract_bank(&mut deps, &env, contract_bank);

    assert!(contract_bank <= BANK_DUST_TOLERANCE);
    assert_eq!(owner_paid, accrued_from_accrual);
}
