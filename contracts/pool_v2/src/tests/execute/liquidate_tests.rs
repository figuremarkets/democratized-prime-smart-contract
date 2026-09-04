//! Tests for Liquidate execute: auth follows liquidation_access (default owner-only;
//! permissionless still requires the owner when unpriceable collateral is load-bearing),
//! borrower must be liquidatable, minimum repay to reach healthy LTV, 2% collateral bonus.

use crate::constants::{
    ATTRIBUTE_BAD_DEBT_UNDERLYING, ATTRIBUTE_DEFICIT_UNDERLYING, ATTRIBUTE_LIQUIDATION_ACCESS,
    ATTRIBUTE_SCALED_AMOUNT,
};
use crate::contract::execute;
use crate::execute::liquidate::{ACTION, ASSERT_OWNER_ERR, ASSERT_OWNER_UNPRICEABLE_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::health::BorrowerHealthV1;
use crate::model::{
    BadDebtLossAllocation, CollateralAssetV1, Denom, LiquidationAccess, RateParamsV1,
    DEFAULT_MAX_LIQUIDATION_STALENESS_SECONDS,
};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{
    get_borrower_collateral, get_contract_state_v1, get_reserve_state_v1, get_scaled_borrow,
};
use crate::tests::fixtures::{oracle_price_expired_for, stale_oracle_price};
use crate::tests::query::common::{CUSTODIAN, OWNER};
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
/// Seized collateral is valued at market (display_price_usd × amount / 10^precision). Band [100%, 102%] of repay → for repay 374 need market value in [374, 381.48];
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

const BORROWER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";
const OTHER: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";
/// "u" prefix => 1 ylds.fcc = 10^6 uylds.fcc.
const LENDING_DENOM: &str = "uylds.fcc";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
/// "nano" prefix => 1 BTC = 10^9 nbtc.figure.se. (These tests use small integer amounts for simplicity.)
const COLLATERAL_DENOM: &str = "nbtc.figure.se";
/// Second supported collateral used to test liquidation when one feed is stale or missing.
const UNRELIABLE_COLLATERAL: &str = "neth.figure.se";
/// 18-decimal collateral for the sc-544110 value_usd band regression.
const WEI_COLLATERAL: &str = "wei.eth.figure.se";
const ONE_WHOLE_18: u128 = 1_000_000_000_000_000_000;

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
        max_liquidation_staleness_seconds: DEFAULT_MAX_LIQUIDATION_STALENESS_SECONDS,
        margin_rate: Decimal256::from_str("0.80").unwrap(),
        liquidation_rate: Decimal256::from_str("0.90").unwrap(),
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128), // 2%
        min_lend: Uint128::new(1),
        min_borrow: Uint128::new(1),
        supported_collateral_assets: vec![
            CollateralAssetV1 {
                asset_id: COLLATERAL_DENOM.to_string(),
                haircut: Some(Decimal256::percent(80)),
            },
            CollateralAssetV1 {
                asset_id: UNRELIABLE_COLLATERAL.to_string(),
                haircut: Some(Decimal256::percent(80)),
            },
        ],
        commit_market_id: None,
        bad_debt_loss_allocation: Default::default(),
        custodian: CUSTODIAN.to_owned(),
        liquidation_access: Default::default(),
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

fn instantiate_msg_with_wei_collateral() -> InstantiateMsg {
    let mut msg = default_instantiate_msg();
    msg.contract_name = "pool-v2-wei-collateral".to_string();
    msg.supported_collateral_assets.push(CollateralAssetV1 {
        asset_id: WEI_COLLATERAL.to_string(),
        haircut: Some(Decimal256::percent(80)),
    });
    msg
}

fn price_entry(price: &str) -> AssetPriceResponseV1 {
    AssetPriceResponseV1::new(Decimal256::from_str(price).unwrap(), 0, u64::MAX)
}

/// Display USD at a mapping precision. `price_usd` is left at zero so a caller that still
/// multiplied the deprecated scaled field would treat the bag as worthless.
fn display_price(display: &str, precision: u32) -> AssetPriceResponseV1 {
    #[allow(deprecated)]
    AssetPriceResponseV1 {
        price_usd: Decimal256::zero(),
        display_price_usd: Decimal256::from_str(display).unwrap(),
        precision,
        as_of_epoch_second: 0,
        expiration_epoch_seconds: u64::MAX,
    }
}

fn seize_all(denom: &str, amount: u128) -> BTreeMap<String, Uint128> {
    let mut m = BTreeMap::new();
    m.insert(denom.to_string(), Uint128::new(amount));
    m
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

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

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

/// Priced dust: 1000 units crash to $0.0005 ($0.50 market, below one lending atom at $1).
/// Debt 700 remains. min_repay clamps to 1; full close with repay 1 books residual 699.
fn setup_priced_dust_borrower(
    allocation: BadDebtLossAllocation,
) -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
) {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let mut msg = instantiate_msg_full_haircut_collateral();
    msg.bad_debt_loss_allocation = allocation;
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

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(1000, COLLATERAL_DENOM)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(700),
        },
    )
    .expect("borrow");

    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.0005"));
    set_oracle_prices(&mut deps.querier, prices);
    (deps, env)
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
fn liquidate_for_custodian_fails() {
    let (mut deps, env, _debt, _) = setup_liquidatable_borrower();
    let min_repay = 374u128;

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(CUSTODIAN),
            &[coin(min_repay, LENDING_DENOM)],
        ),
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

fn set_liquidation_access(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    env: Env,
    access: LiquidationAccess,
) {
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            max_liquidation_staleness_seconds: None,
            liquidation_access: Some(access),
            commit_market_id: None,
            bad_debt_loss_allocation: match access {
                LiquidationAccess::Permissionless => {
                    Some(BadDebtLossAllocation::ImmediateLiquidityIndexHaircut)
                }
                LiquidationAccess::OwnerOnly => None,
            },
            custodian: None,
        },
    )
    .expect("custodian can set liquidation_access");
}

#[test]
fn liquidate_permissionless_allows_non_owner() {
    let (mut deps, env, debt_amount, collateral_amount) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, env.clone(), LiquidationAccess::Permissionless);

    // Send more than debt so we also get a refund BankMsg, both must target OTHER not OWNER.
    let sent = 1000u128;
    assert!(sent > debt_amount, "test sends more than debt");
    let seize_units = 730u128;
    let mut to_seize = BTreeMap::new();
    to_seize.insert(COLLATERAL_DENOM.to_string(), Uint128::new(seize_units));

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OTHER), &[coin(sent, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: to_seize,
        },
    )
    .expect("any sender may liquidate when access is permissionless");

    assert_eq!(
        res.attributes
            .iter()
            .find(|a| a.key == ATTRIBUTE_LIQUIDATION_ACCESS)
            .map(|a| a.value.as_str()),
        Some(LiquidationAccess::Permissionless.as_str())
    );
    assert_eq!(res.messages.len(), 2);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OTHER);
            assert_ne!(to_address.as_str(), OWNER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, COLLATERAL_DENOM);
            assert_eq!(amount[0].amount.u128(), seize_units);
            assert!(seize_units <= collateral_amount);
        }
        _ => panic!("expected first message BankMsg::Send (collateral)"),
    }
    let actual_repay: u128 = res.attributes[3].value.parse().unwrap();
    match &res.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OTHER);
            assert_ne!(to_address.as_str(), OWNER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, LENDING_DENOM);
            assert_eq!(amount[0].amount.u128(), sent - actual_repay);
        }
        _ => panic!("expected second message BankMsg::Send (excess refund)"),
    }
}

#[test]
fn liquidate_permissionless_still_allows_owner() {
    let (mut deps, env, _debt, _) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, env.clone(), LiquidationAccess::Permissionless);

    let min_repay = 374u128;
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .expect("owner may still liquidate when access is permissionless");
}

#[test]
fn liquidate_permissionless_rejects_non_owner_when_unpriceable_has_no_stored_quote() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, env.clone(), LiquidationAccess::Permissionless);
    add_unreliable_dust_then_break_feed(&mut deps, &env, false, true);

    let min_repay = 374u128;
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
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_UNPRICEABLE_ERR
    ));
}

#[test]
fn liquidate_permissionless_rejects_non_owner_when_unpriceable_quote_is_zero() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, env.clone(), LiquidationAccess::Permissionless);
    add_unreliable_dust_then_break_feed(&mut deps, &env, false, false);

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("0"));
    set_oracle_prices(&mut deps.querier, prices);

    let min_repay = 374u128;
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
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_UNPRICEABLE_ERR
    ));
}

#[test]
fn liquidate_permissionless_allows_non_owner_when_unpriceable_dust_is_not_load_bearing() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, env.clone(), LiquidationAccess::Permissionless);
    add_unreliable_collateral_then_expire_beyond_bound(&mut deps, &env, 1);

    let min_repay = 374u128;
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OTHER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .expect("dust of a too-old feed does not disable permissionless");
}

#[test]
fn liquidate_permissionless_rejects_non_owner_when_unpriceable_collateral_is_load_bearing() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, env.clone(), LiquidationAccess::Permissionless);
    // 1000 ETH at last-known $1, haircut 80% → $800. Combined with BTC $664, LTV 600/1464 < 0.9.
    add_unreliable_collateral_then_expire_beyond_bound(&mut deps, &env, 1000);

    let min_repay = 374u128;
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
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_UNPRICEABLE_ERR
    ));
}

#[test]
fn liquidate_permissionless_owner_still_liquidates_when_collateral_unpriceable() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, env.clone(), LiquidationAccess::Permissionless);
    add_unreliable_dust_then_break_feed(&mut deps, &env, false, true);

    let min_repay = 374u128;
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .expect("owner may still liquidate a mixed unpriceable bag under permissionless");
}

#[test]
fn liquidate_permissionless_allows_non_owner_when_stale_within_last_known() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, env.clone(), LiquidationAccess::Permissionless);
    add_unreliable_dust_then_break_feed(&mut deps, &env, true, false);

    let min_repay = 374u128;
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OTHER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .expect("last-known within the bound is still permissionless");
}

#[test]
fn liquidate_succeeds_when_lending_denom_price_is_stale() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let mut prices = HashMap::new();
    prices.insert(
        LENDING_DENOM.to_string(),
        stale_oracle_price(Decimal256::from_str("1.0").unwrap(), env.block.time),
    );
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    set_oracle_prices(&mut deps.querier, prices);

    let min_repay = 374u128;
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .expect("liquidate should succeed using last-known lending denom price");
}

fn add_unreliable_dust_then_break_feed(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    env: &Env,
    unreliable_stale: bool,
    omit_unreliable: bool,
) {
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(1, UNRELIABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add dust of second collateral");

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    if !omit_unreliable {
        if unreliable_stale {
            prices.insert(
                UNRELIABLE_COLLATERAL.to_string(),
                stale_oracle_price(Decimal256::from_str("1.0").unwrap(), env.block.time),
            );
        } else {
            prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.0"));
        }
    }
    set_oracle_prices(&mut deps.querier, prices);
}

fn add_unreliable_collateral_then_expire_beyond_bound(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    env: &Env,
    amount: u128,
) {
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(amount, UNRELIABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add second collateral while its feed is live");

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    prices.insert(
        UNRELIABLE_COLLATERAL.to_string(),
        oracle_price_expired_for(
            Decimal256::from_str("1.0").unwrap(),
            env.block.time,
            DEFAULT_MAX_LIQUIDATION_STALENESS_SECONDS + 1,
        ),
    );
    set_oracle_prices(&mut deps.querier, prices);
}

#[test]
fn liquidate_succeeds_when_one_collateral_price_is_stale() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    add_unreliable_dust_then_break_feed(&mut deps, &env, true, false);

    let min_repay = 374u128;
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .expect("liquidate should succeed with one stale collateral feed");

    assert_eq!(
        res.attributes
            .iter()
            .find(|a| a.key == "action")
            .map(|a| a.value.as_str()),
        Some("liquidate")
    );
    let remaining = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(remaining.amounts.get(UNRELIABLE_COLLATERAL), Some(&1));
}

#[test]
fn liquidate_succeeds_when_one_collateral_price_is_missing() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    add_unreliable_dust_then_break_feed(&mut deps, &env, false, true);

    let min_repay = 374u128;
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: collateral_to_seize_success(),
        },
    )
    .expect("liquidate should succeed with one missing collateral feed");
}

#[test]
fn liquidate_fails_when_seizing_unpriceable_collateral() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    add_unreliable_dust_then_break_feed(&mut deps, &env, false, true);

    let mut seize = collateral_to_seize_success();
    seize.insert(UNRELIABLE_COLLATERAL.to_string(), Uint128::new(1));

    let min_repay = 374u128;
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Cannot seize unpriceable collateral"));
            assert!(message.contains(UNRELIABLE_COLLATERAL));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

/// A stored zero does not count as last-known. Seizing it beside a valued asset would
/// add $0 to the band, so the bonus cap would not stop the sweep.
#[test]
fn liquidate_fails_when_seizing_zero_price_collateral() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    add_unreliable_dust_then_break_feed(&mut deps, &env, false, false);

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("0"));
    set_oracle_prices(&mut deps.querier, prices);

    let mut seize = collateral_to_seize_success();
    seize.insert(UNRELIABLE_COLLATERAL.to_string(), Uint128::new(1));

    let min_repay = 374u128;
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Cannot seize unpriceable collateral"));
            assert!(message.contains(UNRELIABLE_COLLATERAL));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn liquidate_succeeds_when_seizing_stale_collateral() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    add_unreliable_dust_then_break_feed(&mut deps, &env, true, false);

    // 455 of A at 0.83 = 377.65; + 1 of B at last-known 1.0 = 378.65; band for repay 374 is [374, 381.48].
    let mut seize = collateral_to_seize_success();
    seize.insert(UNRELIABLE_COLLATERAL.to_string(), Uint128::new(1));

    let min_repay = 374u128;
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize,
        },
    )
    .expect("liquidate should succeed seizing stale-priced collateral at last-known");

    let remaining = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert!(!remaining.amounts.contains_key(UNRELIABLE_COLLATERAL));
}

#[test]
fn liquidate_fails_when_seizing_collateral_beyond_staleness_bound() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    add_unreliable_dust_then_break_feed(&mut deps, &env, true, false);

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.83"));
    prices.insert(
        UNRELIABLE_COLLATERAL.to_string(),
        oracle_price_expired_for(
            Decimal256::from_str("1.0").unwrap(),
            env.block.time,
            DEFAULT_MAX_LIQUIDATION_STALENESS_SECONDS + 1,
        ),
    );
    set_oracle_prices(&mut deps.querier, prices);

    let mut seize = collateral_to_seize_success();
    seize.insert(UNRELIABLE_COLLATERAL.to_string(), Uint128::new(1));

    let min_repay = 374u128;
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(min_repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Cannot seize unpriceable collateral"));
            assert!(message.contains(UNRELIABLE_COLLATERAL));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn liquidate_fails_when_all_collateral_has_no_stored_price() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
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
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("No priceable collateral"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
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

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(1000, COLLATERAL_DENOM)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

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

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(1000, COLLATERAL_DENOM)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

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
    let bad_debt_amt = 50u128;
    let d = Decimal256::from_ratio(Uint128::from(bad_debt_amt), Uint128::one());
    let scaled = Decimal256::from_ratio(Uint128::from(eff.total_scaled_liquidity), Uint128::one());
    let exp_liquidity_index = l.checked_sub(d).unwrap().checked_div(scaled).unwrap();

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

// --- Full close: waive 100% floor when seizure empties the collateral map ---

/// Invariant: a full seizure with residual debt zeros `scaled_borrow` in the same
/// transaction (empty map + leftover debt is not a reachable post-state).
#[test]
fn liquidate_full_close_priced_dust_books_deficit() {
    let (mut deps, env) = setup_priced_dust_borrower(BadDebtLossAllocation::DeferredToDeficit);

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize_all(COLLATERAL_DENOM, 1000),
        },
    )
    .expect("full close of priced dust");

    let bad_debt = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_BAD_DEBT_UNDERLYING)
        .expect("bad_debt_underlying attribute");
    assert_eq!(bad_debt.value, "699");
    let deficit_attr = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_DEFICIT_UNDERLYING)
        .expect("deficit_underlying attribute");
    assert_eq!(deficit_attr.value, "699");

    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    assert!(get_borrower_collateral(deps.as_ref().storage, BORROWER)
        .unwrap()
        .amounts
        .is_empty());
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 699);
    assert_reserve_assets_liabilities_tie_out(deps.as_ref().storage, "after dust full close")
        .unwrap();
}

#[test]
fn liquidate_full_close_priced_dust_immediate_haircut() {
    let (mut deps, env) =
        setup_priced_dust_borrower(BadDebtLossAllocation::ImmediateLiquidityIndexHaircut);

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let eff =
        compute_effective_reserve(deps.as_ref().storage, env.block.time, &contract.rate_params)
            .expect("effective reserve before liquidate");
    let l = eff.total_liquidity().unwrap();
    let bad_debt_amt = 699u128;
    let d = Decimal256::from_ratio(Uint128::from(bad_debt_amt), Uint128::one());
    let scaled = Decimal256::from_ratio(Uint128::from(eff.total_scaled_liquidity), Uint128::one());
    let exp_liquidity_index = l.checked_sub(d).unwrap().checked_div(scaled).unwrap();

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize_all(COLLATERAL_DENOM, 1000),
        },
    )
    .expect("full close of priced dust with immediate haircut");

    let bad_debt = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_BAD_DEBT_UNDERLYING)
        .expect("bad_debt_underlying attribute");
    assert_eq!(bad_debt.value, "699");
    assert_eq!(
        res.attributes
            .iter()
            .find(|a| a.key == ATTRIBUTE_DEFICIT_UNDERLYING)
            .map(|a| a.value.as_str()),
        Some("0")
    );

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 0);
    assert_eq!(reserve.liquidity_index, exp_liquidity_index);
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    assert!(get_borrower_collateral(deps.as_ref().storage, BORROWER)
        .unwrap()
        .amounts
        .is_empty());
    assert_reserve_assets_liabilities_tie_out(
        deps.as_ref().storage,
        "after dust full close immediate haircut",
    )
    .unwrap();
}

/// Safety property: full close does not waive the bonus cap. A valuable bag cannot be
/// emptied by a repay whose bonus multiple is below the bag's market value.
#[test]
fn liquidate_full_close_valuable_bag_rejected_by_bonus_cap() {
    let (mut deps, env, _, collateral_amount) = setup_liquidatable_borrower();
    let repay_amount = 374u128; // meets min repay so the band, not the floor, is what rejects

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(OWNER),
            &[coin(repay_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize_all(COLLATERAL_DENOM, collateral_amount),
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
fn liquidate_partial_seizure_still_requires_100_percent_floor() {
    let (mut deps, env, _, _) = setup_liquidatable_borrower();
    let repay_amount = 374u128;
    // Strict subset: 1 unit at $0.83 is below 100% of repay 374; not a full close, so the floor binds.
    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(OWNER),
            &[coin(repay_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize_all(COLLATERAL_DENOM, 1),
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

/// Owner attaches the entire debt and seizes everything: no deficit, no index haircut.
#[test]
fn liquidate_full_close_with_full_debt_repayment_books_no_deficit() {
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

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(1000, COLLATERAL_DENOM)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(700),
        },
    )
    .expect("borrow");

    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.70"));
    set_oracle_prices(&mut deps.querier, prices);

    let index_before = get_reserve_state_v1(deps.as_ref().storage)
        .unwrap()
        .liquidity_index;

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(700, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize_all(COLLATERAL_DENOM, 1000),
        },
    )
    .expect("full close with full debt repayment");

    assert!(res
        .attributes
        .iter()
        .all(|a| a.key != ATTRIBUTE_BAD_DEBT_UNDERLYING));
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    assert!(get_borrower_collateral(deps.as_ref().storage, BORROWER)
        .unwrap()
        .amounts
        .is_empty());
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 0);
    assert_eq!(reserve.liquidity_index, index_before);
    assert_reserve_assets_liabilities_tie_out(
        deps.as_ref().storage,
        "after full-close full repayment",
    )
    .unwrap();
}

/// 3 whole 18-decimal tokens at $0.40 display = $1.20 market, above the 1-atom bonus cap ($1.02).
/// Deprecated scaled `price_usd` is zero in the fixture; the band must use `value_usd`.
#[test]
fn liquidate_full_close_18_decimal_cheap_display_rejected_by_bonus_cap() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();

    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        instantiate_msg_with_wei_collateral(),
    )
    .expect("instantiate");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(1000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    let wei_amount = 3 * ONE_WHOLE_18;
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(WEI_COLLATERAL.to_string(), display_price("1.0", 18));
    set_oracle_prices(&mut deps.querier, prices.clone());

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(wei_amount, WEI_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(1),
        },
    )
    .expect("borrow");

    prices.insert(WEI_COLLATERAL.to_string(), display_price("0.40", 18));
    set_oracle_prices(&mut deps.querier, prices);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize_all(WEI_COLLATERAL, wei_amount),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("exceeds allowed maximum")
                    || message.contains("borrower protection"),
                "band must use value_usd (not truncated price_usd): {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
