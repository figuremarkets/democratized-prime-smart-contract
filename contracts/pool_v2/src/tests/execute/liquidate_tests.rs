//! Tests for Liquidate execute: contract owner only, borrower must be liquidatable, minimum repay to reach
//! healthy LTV, 2% collateral bonus; failures for non-owner, no debt, not liquidatable, insufficient repay.

use crate::constants::{
    ATTRIBUTE_BAD_DEBT_UNDERLYING, ATTRIBUTE_DEFICIT_UNDERLYING, ATTRIBUTE_SCALED_AMOUNT,
};
use crate::contract::execute;
use crate::execute::liquidate::{ACTION, ASSERT_OWNER_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::health::BorrowerHealthV1;
use crate::model::{BadDebtLossAllocation, CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{
    get_borrower_collateral, get_contract_state_v1, get_reserve_state_v1, get_scaled_borrow,
};
use crate::tests::fixtures::stale_oracle_price;
use crate::tests::reserve_invariant::assert_reserve_assets_liabilities_tie_out;
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use crate::utils::{
    compute_effective_reserve, get_asset_prices_for_borrower, get_borrower_health,
    scaled_to_underlying_borrow,
};
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256, Env,
    MemoryStorage, OwnedDeps, QuerierResult, SystemError, SystemResult, Timestamp, Uint128,
    WasmQuery,
};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

/// With debt 600 and collateral price 0.83 (haircutted value 664), min repay = 374 (formula: (D - margin*C)/(1 - bonus*margin)).
/// Seized collateral is valued at market (price × amount). Band [100%, 102%] of repay → for repay 374 need market value in [374, 381.48];
/// at price 0.83 that is ~451–460 units. Use 455 (market value 377.65).
fn collateral_to_seize_success() -> BTreeMap<String, Uint128> {
    let mut m = BTreeMap::new();
    m.insert(COLLATERAL_DENOM.to_string(), Uint128::new(455));
    m
}

fn collateral_to_seize_min() -> BTreeMap<String, Uint128> {
    let mut m = BTreeMap::new();
    m.insert(COLLATERAL_DENOM.to_string(), Uint128::new(1));
    m
}

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const BORROWER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";
const OTHER: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";
/// "u" prefix => 1 ylds.fcc = 10^6 uylds.fcc.
const LENDING_DENOM: &str = "uylds.fcc";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
/// "nano" prefix => 1 BTC = 10^9 nbtc.figure.se. (These tests use small integer amounts for simplicity.)
const COLLATERAL_DENOM: &str = "nbtc.figure.se";

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
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128), // 2%
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

/// Same as [`default_instantiate_msg`] but collateral has **no haircut** so underwater liquidation can
/// seize 100% of units in one call (seizure band is vs full market value of collateral).
fn instantiate_msg_full_haircut_collateral() -> InstantiateMsg {
    let mut msg = default_instantiate_msg();
    msg.contract_name = "pool-v2-bad-debt-test".to_string();
    msg.supported_collateral_assets = vec![CollateralAssetV1 {
        asset_id: COLLATERAL_DENOM.to_string(),
        haircut: None,
    }];
    msg
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

/// Setup: pool with lent supply; borrower with collateral and debt such that LTV = 0.9 (liquidatable).
/// Borrow at LTV < 80% first (so borrow succeeds), then lower collateral price so LTV becomes 90%.
/// - Collateral 1000, haircut 0.8. At price 1.0, value = 800. Borrow 600 -> LTV 75% (healthy).
/// - Then set collateral price to 0.6667 so value = 666.67, LTV = 600/666.67 = 90% (liquidatable).
fn setup_liquidatable_borrower() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
    u128,
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

    let lend_amount = 1_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(lend_amount, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend should succeed");

    let collateral_amount = 1000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(collateral_amount, COLLATERAL_DENOM)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral should succeed");

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

    let debt_amount = 600u128; // LTV = 600 / (1000*0.8) = 0.75 (healthy)
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(debt_amount),
        },
    )
    .expect("borrow should succeed");

    // Lower collateral price so LTV is clearly >= 90%. Use 0.83 -> value = 664, LTV = 600/664 > 0.9.
    // (0.8333375 gave value 666.67 and LTV exactly 0.9; rounding can make it just under.)
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    set_oracle_prices(&mut deps.querier, prices);

    (deps, env, debt_amount, collateral_amount)
}

#[test]
fn liquidate_non_owner_fails() {
    let (mut deps, env, _debt, _) = setup_liquidatable_borrower();
    let min_repay = 374u128; // min to bring LTV to healthy (debt 600, collateral value 664, price 0.83)

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OTHER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn liquidate_fails_when_oracle_price_is_stale() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let mut prices = HashMap::new();
    prices.insert(
        LENDING_DENOM.to_string(),
        stale_oracle_price(Decimal256::from_str("1.0").unwrap(), env.block.time),
    );
    prices.insert(
        COLLATERAL_DENOM.to_string(),
        stale_oracle_price(Decimal256::from_str("0.83").unwrap(), env.block.time),
    );
    set_oracle_prices(&mut deps.querier, prices);

    let min_repay = 374u128;
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::StalePriceDataError { .. } => {}
        _ => panic!("expected StalePriceDataError, got {:?}", err),
    }
}

#[test]
fn liquidate_borrower_with_no_debt_fails() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let reserve =
        compute_effective_reserve(deps.as_ref().storage, env.block.time, &contract.rate_params)
            .unwrap();
    let scaled = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    let debt = scaled_to_underlying_borrow(scaled, reserve.borrow_index).unwrap();
    assert!(debt >= 600, "setup should have debt");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(debt, LENDING_DENOM)]),
        ExecuteMsg::Repay {},
    )
    .expect("repay to clear debt");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(100, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_min(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("no debt"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn liquidate_healthy_borrower_fails() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let reserve =
        compute_effective_reserve(deps.as_ref().storage, env.block.time, &contract.rate_params)
            .unwrap();
    let scaled = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    let debt = scaled_to_underlying_borrow(scaled, reserve.borrow_index).unwrap();
    let collateral = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83")); // LTV > 90% (liquidatable)
    set_oracle_prices(&mut deps.querier, prices);
    let asset_prices = get_asset_prices_for_borrower(
        &deps.as_ref().querier,
        &env.block.time,
        &contract,
        &collateral,
    )
    .unwrap();
    let (health, _) = get_borrower_health(
        &contract,
        &contract.supported_collateral_assets,
        &asset_prices,
        &collateral,
        Uint128::from(debt),
    )
    .unwrap();
    assert_eq!(health, BorrowerHealthV1::Liquidatable);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(200, LENDING_DENOM)]),
        ExecuteMsg::Repay {},
    )
    .expect("partial repay to make position healthy");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(100, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_min(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("not liquidatable"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn liquidate_below_min_repay_fails() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(100, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("below minimum") || message.contains("bring LTV to healthy"),
                "message: {}",
                message
            );
            assert!(
                message.contains("374"),
                "message should mention required minimum 374, got: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn liquidate_succeeds_and_sends_collateral_to_owner() {
    let (mut deps, env, _debt, collateral_amount) = setup_liquidatable_borrower();
    let scaled_before = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();

    let repay_amount = 374u128;
    let seize_units = 455u128; // market value in [100%, 102%] of repay at price 0.83 (455*0.83 ≈ 377.65)
    let mut to_seize = BTreeMap::new();
    to_seize.insert(COLLATERAL_DENOM.to_string(), Uint128::new(seize_units));
    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(OWNER),
            &[coin(repay_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: to_seize,
        },
    )
    .expect("liquidate should succeed");

    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].value, OWNER);
    assert_eq!(res.attributes[2].value, BORROWER);
    assert_eq!(res.attributes[3].value, repay_amount.to_string());

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, COLLATERAL_DENOM);
            assert_eq!(
                amount[0].amount.u128(),
                seize_units,
                "liquidator chose {} units",
                seize_units
            );
            assert!(
                seize_units <= collateral_amount,
                "cannot seize more than borrower had"
            );
        }
        _ => panic!("expected BankMsg::Send"),
    }

    let scaled_after = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(res.attributes[4].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(
        res.attributes[4].value,
        (scaled_before - scaled_after).to_string()
    );
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert!(scaled_after < scaled_before);
    let collateral_after = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(
        collateral_after.amounts.get(COLLATERAL_DENOM),
        Some(&(collateral_amount - seize_units)),
        "borrower had {}, we seized {}",
        collateral_amount,
        seize_units
    );
    assert_reserve_assets_liabilities_tie_out(deps.as_ref().storage, "after liquidate").unwrap();
}

/// Full debt repayment via liquidation after interest has accrued: without the double-floor fix,
/// scaled_repay = floor(debt_underlying/borrow_index) can be < scaled_debt, leaving dust. Advance
/// time so borrow_index > 1, then liquidate with sent >= debt_underlying; assert scaled_borrow == 0.
#[test]
fn liquidate_full_debt_after_interest_accrual_clears_scaled_debt() {
    let (mut deps, mut env, _debt_amount, collateral_amount) = setup_liquidatable_borrower();
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

    // Min repay to satisfy LTV (unchanged by time for this setup). Send full debt to clear position.
    let sent = debt_underlying;
    // Market value of seized collateral in [100%, 102%] of debt. After 1y accrual debt > 600 (e.g. 619). Price 0.83 → need ~746–761 units. Use 755 (755*0.83 ≈ 626.65).
    let seize_units = 755u128;
    let mut to_seize = BTreeMap::new();
    to_seize.insert(COLLATERAL_DENOM.to_string(), Uint128::new(seize_units));

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(sent, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: to_seize,
        },
    )
    .expect("liquidate should succeed");

    let actual_repay: u128 = res.attributes[3].value.parse().unwrap();
    assert_eq!(actual_repay, debt_underlying);
    assert_eq!(res.attributes[4].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[4].value, scaled_debt.to_string());
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    let scaled_after = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(
        scaled_after, 0,
        "full liquidation must clear all scaled debt (no dust); scaled_debt was {}, borrow_index {}",
        scaled_debt, reserve.borrow_index
    );
    let collateral_after = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(
        collateral_after.amounts.get(COLLATERAL_DENOM),
        Some(&(collateral_amount - seize_units)),
    );
    assert_reserve_assets_liabilities_tie_out(
        deps.as_ref().storage,
        "after liquidate (full debt after accrual)",
    )
    .unwrap();
}

/// When the contract owner sends more than the borrower's total debt, only debt is applied and excess is refunded
/// (BankMsg::Send back to owner). Same behavior as Repay.
#[test]
fn liquidate_excess_repay_refunded() {
    let (mut deps, env, debt_amount, collateral_amount) = setup_liquidatable_borrower();
    let scaled_before = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert!(scaled_before > 0);

    // Send more than debt (e.g. 1000); actual_repay = min(sent, debt) = debt (~600).
    let sent = 1000u128;
    assert!(sent > debt_amount, "test sends more than debt");
    // Market value in [600, 612]. At price 0.83 need ~723–737 units. Use 730.
    let seize_units = 730u128;
    let mut to_seize = BTreeMap::new();
    to_seize.insert(COLLATERAL_DENOM.to_string(), Uint128::new(seize_units));

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(sent, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: to_seize,
        },
    )
    .expect("liquidate should succeed");

    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].value, OWNER);
    assert_eq!(res.attributes[2].value, BORROWER);
    // Actual repay is capped at debt.
    let actual_repay: u128 = res.attributes[3].value.parse().unwrap();
    assert_eq!(actual_repay, debt_amount);
    assert_eq!(res.attributes[4].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[4].value, scaled_before.to_string());
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    let excess = sent - actual_repay;
    assert!(excess > 0);

    // First message: collateral to the liquidator; second: excess lending tokens refund to liquidator.
    assert_eq!(res.messages.len(), 2);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, COLLATERAL_DENOM);
            assert_eq!(amount[0].amount.u128(), seize_units);
        }
        _ => panic!("expected first message BankMsg::Send (collateral)"),
    }
    match &res.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, LENDING_DENOM);
            assert_eq!(
                amount[0].amount.u128(),
                excess,
                "excess lending tokens must be refunded"
            );
        }
        _ => panic!("expected second message BankMsg::Send (excess refund)"),
    }

    let scaled_after = get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(scaled_after, 0, "debt should be fully repaid");
    let collateral_after = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(
        collateral_after.amounts.get(COLLATERAL_DENOM),
        Some(&(collateral_amount - seize_units)),
    );
    assert_reserve_assets_liabilities_tie_out(
        deps.as_ref().storage,
        "after liquidate (excess refund)",
    )
    .unwrap();
}

#[test]
fn liquidate_insufficient_collateral_value_fails() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let repay_amount = 374u128; // min repay with price 0.83 (collateral value 664); must be >= min to reach this test
    let mut too_little = BTreeMap::new();
    too_little.insert(COLLATERAL_DENOM.to_string(), Uint128::new(400)); // market value 400*0.83 = 332 < 374 (100% of repay)

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(OWNER),
            &[coin(repay_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: too_little,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("below required") || message.contains("100%"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn liquidate_excess_collateral_value_fails() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let repay_amount = 374u128;
    let mut too_much = BTreeMap::new();
    too_much.insert(COLLATERAL_DENOM.to_string(), Uint128::new(600)); // market value 600*0.83 = 498 > 102% of 374 (~381.48)

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(OWNER),
            &[coin(repay_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: too_much,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("exceeds allowed maximum")
                    || message.contains("borrower protection"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn liquidate_empty_collateral_to_seize_fails() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let empty: BTreeMap<String, Uint128> = BTreeMap::new();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(374, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: empty,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("collateral_to_seize"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn liquidate_no_funds_fails() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_min(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::InvalidFundsError { .. } | ContractError::IllegalArgumentError { .. } => {}
        _ => panic!("expected funds error, got {:?}", err),
    }
}

/// When min_repay_lending would round to 0 (e.g. min_repay_value_usd from formula is 0), we clamp to 1.
/// Repay amount 0 must be rejected (below minimum required 1).
#[test]
fn liquidate_repay_amount_zero_fails() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(0, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_min(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("below minimum") || message.contains("minimum required"),
                "expected minimum-amount error, got: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

// --- Bad-debt liquidation (deferred vs immediate index haircut) ---

/// Underwater borrower: the contract owner repays up to collateral value (650), seizes all collateral; residual
/// scaled debt is written off to **`deficit_underlying`** (50) and borrower scaled borrow is cleared.
/// Underwater full seizure with partial repay (historical “phantom debt” reproducer).
#[test]
fn liquidate_bad_debt_books_deficit_and_clears_scaled_borrow() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();

    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        instantiate_msg_full_haircut_collateral(),
    )
    .expect("instantiate");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(1000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(1000, COLLATERAL_DENOM)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(700),
        },
    )
    .expect("borrow");

    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.65"));
    set_oracle_prices(&mut deps.querier, prices);

    let mut all_collateral = BTreeMap::new();
    all_collateral.insert(COLLATERAL_DENOM.to_string(), Uint128::new(1000));

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(650, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: all_collateral,
        },
    )
    .expect("liquidate underwater");

    let bad_debt = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_BAD_DEBT_UNDERLYING)
        .expect("bad_debt_underlying attribute");
    assert_eq!(bad_debt.value, "50");
    let deficit_attr = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_DEFICIT_UNDERLYING)
        .expect("deficit_underlying attribute");
    assert_eq!(deficit_attr.value, "50");

    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 50);

    let collateral_after = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert!(collateral_after.amounts.is_empty());

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_reserve_assets_liabilities_tie_out(deps.as_ref().storage, "after bad-debt liquidate")
        .unwrap();

    let mut dummy_seize = BTreeMap::new();
    dummy_seize.insert(COLLATERAL_DENOM.to_string(), Uint128::new(1));
    let second_err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(50, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: dummy_seize,
        },
    )
    .unwrap_err();
    match &second_err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("no debt") || message.contains("no collateral"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", second_err),
    }
}

/// Same underwater scenario as `liquidate_bad_debt_books_deficit_and_clears_scaled_borrow`, but
/// **`bad_debt_loss_allocation: ImmediateLiquidityIndexHaircut`**: no `deficit_underlying`; index cut.
#[test]
fn liquidate_bad_debt_immediate_haircut_skips_deficit() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();

    let mut msg = instantiate_msg_full_haircut_collateral();
    msg.bad_debt_loss_allocation = BadDebtLossAllocation::ImmediateLiquidityIndexHaircut;
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(1000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(1000, COLLATERAL_DENOM)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(700),
        },
    )
    .expect("borrow");

    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.65"));
    set_oracle_prices(&mut deps.querier, prices);

    let mut all_collateral = BTreeMap::new();
    all_collateral.insert(COLLATERAL_DENOM.to_string(), Uint128::new(1000));

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let eff =
        compute_effective_reserve(deps.as_ref().storage, env.block.time, &contract.rate_params)
            .expect("effective reserve before liquidate");
    let l = eff.total_liquidity().unwrap();
    let li0 = eff.liquidity_index;
    let bad_debt_amt = 50u128;
    let d = Decimal256::from_ratio(Uint128::from(bad_debt_amt), Uint128::one());
    let exp_liquidity_index = li0
        .checked_mul(l.checked_sub(d).unwrap().checked_div(l).unwrap())
        .unwrap();

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(650, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: all_collateral,
        },
    )
    .expect("liquidate underwater immediate haircut");

    let bad_debt = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_BAD_DEBT_UNDERLYING)
        .expect("bad_debt_underlying attribute");
    assert_eq!(bad_debt.value, "50");
    let deficit_attr = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_DEFICIT_UNDERLYING)
        .expect("deficit_underlying attribute");
    assert_eq!(deficit_attr.value, "0");

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 0);
    assert_eq!(
        reserve.liquidity_index, exp_liquidity_index,
        "liquidity_index must match apply_pro_rata using effective L at liquidate entry"
    );
    assert_reserve_assets_liabilities_tie_out(deps.as_ref().storage, "after immediate haircut")
        .unwrap();
}
