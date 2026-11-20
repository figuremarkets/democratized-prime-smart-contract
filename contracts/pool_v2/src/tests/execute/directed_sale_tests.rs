//! Directed collateral sales: one offer per borrower, filled by a subset `Liquidate`.

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_APPLIED_JSON, ATTRIBUTE_DIRECTED_SALE,
    ATTRIBUTE_EXPIRES_AT, ATTRIBUTE_SWEPT_COLLATERAL_JSON, ATTRIBUTE_VERSION,
    DIRECTED_SALE_MAX_LIFETIME_SECONDS,
};
use crate::contract::{execute, query};
use crate::execute::adjust_directed_collateral_sale::ACTION;
use crate::execute::liquidate::{ASSERT_LIQUIDATOR_ERR, ASSERT_OWNER_UNPRICEABLE_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{
    BadDebtLossAllocation, BorrowerPositionResponseV1, CollateralAssetV1, Denom,
    DirectedSaleOfferV1, LiquidationAccess, RateParamsV1,
    DEFAULT_MAX_LIQUIDATION_STALENESS_SECONDS,
};
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, RepoTokenConfig};
use crate::storage::{
    get_borrower_collateral, get_directed_sale_offer, get_directed_sale_version,
    get_reserve_state_v1, get_scaled_borrow, set_directed_sale_offer,
};
use crate::tests::fixtures::{oracle_price_expired_for, stale_oracle_price};
use crate::tests::query::common::{CUSTODIAN, OWNER};
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, coins, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256,
    Env, Int128, MemoryStorage, OwnedDeps, QuerierResult, SystemError, SystemResult, Timestamp,
    Uint128, WasmQuery,
};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

const BORROWER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";
const OTHER: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";
const LENDING_DENOM: &str = "uylds.fcc";
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
const COLLATERAL_DENOM: &str = "nbtc.figure.se";
const UNRELIABLE_COLLATERAL: &str = "neth.figure.se";

fn default_instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        contract_name: "pool-v2-directed-sale".to_string(),
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
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128),
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
        liquidator: OWNER.to_owned(),
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

fn prices_with_collateral(collateral_price: &str) -> PriceMapResponse {
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry(collateral_price));
    prices
}

type Deps = OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>;

/// Collateral 1000, haircut 80%. Price 1.0 → healthy (LTV 75% at debt 600).
/// Price 0.88 → unhealthy (LTV ~85%). Price 0.83 → liquidatable (LTV ~90%).
fn setup_borrower(debt: u128, collateral_price: &str) -> (Deps, Env) {
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
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(1_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");
    set_oracle_prices(&mut deps.querier, prices_with_collateral("1.0"));
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
            amount: Uint128::new(debt),
        },
    )
    .expect("borrow");
    if collateral_price != "1.0" {
        set_oracle_prices(&mut deps.querier, prices_with_collateral(collateral_price));
    }
    (deps, env)
}

fn setup_healthy_borrower() -> (Deps, Env) {
    setup_borrower(600, "1.0")
}

fn setup_unhealthy_borrower() -> (Deps, Env) {
    setup_borrower(600, "0.88")
}

fn setup_liquidatable_borrower() -> (Deps, Env) {
    setup_borrower(600, "0.83")
}

fn seize(amount: u128) -> BTreeMap<String, Uint128> {
    let mut m = BTreeMap::new();
    m.insert(COLLATERAL_DENOM.to_string(), Uint128::new(amount));
    m
}

fn two_asset_amounts(amount: u128) -> BTreeMap<String, Uint128> {
    let mut amounts = seize(amount);
    amounts.insert(UNRELIABLE_COLLATERAL.to_string(), Uint128::new(amount));
    amounts
}

/// Healthy borrower holding 1000 `nbtc` at $1 and 400 `neth` at $1.50.
fn setup_two_asset_borrower() -> (Deps, Env) {
    let (mut deps, env) = setup_healthy_borrower();
    let mut prices = prices_with_collateral("1.0");
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.5"));
    set_oracle_prices(&mut deps.querier, prices);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(400, UNRELIABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add second collateral");
    (deps, env)
}

fn future(env: &Env) -> Timestamp {
    env.block.time.plus_seconds(3_600)
}

fn delta(amount: i128) -> BTreeMap<String, Int128> {
    let mut m = BTreeMap::new();
    m.insert(COLLATERAL_DENOM.to_string(), Int128::from(amount));
    m
}

fn next_version(deps: &Deps, sender: &str) -> u64 {
    get_directed_sale_version(deps.as_ref().storage, sender).unwrap() + 1
}

fn adjust(
    deps: &mut Deps,
    env: &Env,
    sender: &str,
    adjustments: BTreeMap<String, Int128>,
    expires_at: Timestamp,
) -> Result<cosmwasm_std::Response, ContractError> {
    let version = next_version(deps, sender);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(sender), &[]),
        ExecuteMsg::AdjustDirectedCollateralSale {
            adjustments,
            expires_at,
            version,
        },
    )
}

fn set_offer(
    deps: &mut Deps,
    env: &Env,
    sender: &str,
    collateral: BTreeMap<String, Uint128>,
    expires_at: Timestamp,
) -> Result<cosmwasm_std::Response, ContractError> {
    let adjustments = collateral
        .into_iter()
        .map(|(k, v)| (k, Int128::from(v.u128() as i128)))
        .collect();
    adjust(deps, env, sender, adjustments, expires_at)
}

fn repay(
    deps: &mut Deps,
    env: &Env,
    amount: u128,
) -> Result<cosmwasm_std::Response, ContractError> {
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(amount, LENDING_DENOM)]),
        ExecuteMsg::Repay {},
    )
}

fn borrow_more(
    deps: &mut Deps,
    env: &Env,
    amount: u128,
) -> Result<cosmwasm_std::Response, ContractError> {
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(amount),
        },
    )
}

fn liquidate(
    deps: &mut Deps,
    env: &Env,
    sender: &str,
    repay: u128,
    collateral_to_seize: BTreeMap<String, Uint128>,
) -> Result<cosmwasm_std::Response, ContractError> {
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(sender), &[coin(repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize,
            min_collateral_out: None,
        },
    )
}

fn liquidate_fit(
    deps: &mut Deps,
    env: &Env,
    repay: u128,
    collateral_to_seize: BTreeMap<String, Uint128>,
    min_collateral_out: BTreeMap<String, Uint128>,
) -> Result<cosmwasm_std::Response, ContractError> {
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(repay, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize,
            min_collateral_out: Some(min_collateral_out),
        },
    )
}

fn mins(pairs: &[(&str, u128)]) -> BTreeMap<String, Uint128> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), Uint128::new(*v)))
        .collect()
}

fn bank_amounts(res: &cosmwasm_std::Response) -> Vec<Vec<cosmwasm_std::Coin>> {
    res.messages
        .iter()
        .map(|msg| match &msg.msg {
            CosmosMsg::Bank(BankMsg::Send { amount, .. }) => amount.clone(),
            other => panic!("expected bank send, got {:?}", other),
        })
        .collect()
}

fn position(deps: &Deps, env: &Env) -> BorrowerPositionResponseV1 {
    let bin = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::GetBorrowerPosition {
            address: BORROWER.to_string(),
        },
    )
    .expect("query");
    from_json(bin).unwrap()
}

fn attr<'a>(res: &'a cosmwasm_std::Response, key: &str) -> &'a str {
    res.attributes
        .iter()
        .find(|a| a.key == key)
        .unwrap_or_else(|| panic!("missing attribute {}", key))
        .value
        .as_str()
}

fn set_liquidation_access(deps: &mut Deps, env: &Env, access: LiquidationAccess) {
    execute(
        deps.as_mut(),
        env.clone(),
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
                LiquidationAccess::LiquidatorOnly => None,
            },
            custodian: None,
            liquidator: None,
        },
    )
    .expect("set liquidation_access");
}

fn illegal_contains(err: ContractError, needle: &str) {
    match err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains(needle), "message: {}", message);
        }
        other => panic!("expected IllegalArgumentError, got {:?}", other),
    }
}

#[test]
fn o1_o2_o3_offer_increase_decrease_cancel() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    let res = set_offer(&mut deps, &env, BORROWER, seize(400), exp).expect("increase");
    assert_eq!(attr(&res, ATTRIBUTE_ACTION_NAME), ACTION);
    assert_eq!(attr(&res, ATTRIBUTE_EXPIRES_AT), exp.nanos().to_string());
    assert_eq!(attr(&res, ATTRIBUTE_VERSION), "1");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .expect("stored");
    assert_eq!(stored.amounts, seize(400));
    assert_eq!(stored.expires_at, exp);
    assert_eq!(position(&deps, &env).directed_sale_version, 1);

    let exp2 = env.block.time.plus_seconds(7_200);
    adjust(&mut deps, &env, BORROWER, delta(-200), exp2).expect("decrease");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    assert_eq!(stored.amounts, seize(200));
    assert_eq!(stored.expires_at, exp, "decrease keeps the existing expiry");
    assert_eq!(
        get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap(),
        2
    );

    let res = set_offer(
        &mut deps,
        &env,
        BORROWER,
        BTreeMap::new(),
        Timestamp::from_nanos(0),
    )
    .expect("cancel");
    assert_eq!(attr(&res, ATTRIBUTE_APPLIED_JSON), "{}");
    assert_eq!(attr(&res, ATTRIBUTE_EXPIRES_AT), "0");
    assert_eq!(attr(&res, ATTRIBUTE_VERSION), "3");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    assert_eq!(position(&deps, &env).directed_sale_version, 3);
}

#[test]
fn o4_other_addresses_cannot_mutate_the_borrower_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(400), future(&env)).unwrap();
    for sender in [OTHER, OWNER, CUSTODIAN] {
        let err = set_offer(&mut deps, &env, sender, seize(400), future(&env)).unwrap_err();
        illegal_contains(err, "no debt");
        set_offer(
            &mut deps,
            &env,
            sender,
            BTreeMap::new(),
            Timestamp::from_nanos(1),
        )
        .unwrap();
        assert_eq!(
            get_directed_sale_offer(deps.as_ref().storage, BORROWER)
                .unwrap()
                .unwrap()
                .amounts,
            seize(400)
        );
    }
}

#[test]
fn o5_o6_set_rejects_invalid_offers_and_zero_map_cancels() {
    let (mut deps, env) = setup_healthy_borrower();
    let err = set_offer(&mut deps, &env, BORROWER, seize(1001), future(&env)).unwrap_err();
    illegal_contains(err, "Insufficient collateral");

    let mut unknown = BTreeMap::new();
    unknown.insert("no.such".to_string(), Uint128::new(1));
    let err = set_offer(&mut deps, &env, BORROWER, unknown, future(&env)).unwrap_err();
    illegal_contains(err, "Unsupported collateral");

    let err = set_offer(&mut deps, &env, BORROWER, seize(10), env.block.time).unwrap_err();
    illegal_contains(err, "expires_at");

    set_offer(&mut deps, &env, BORROWER, seize(10), future(&env)).unwrap();
    let mut zeros = BTreeMap::new();
    zeros.insert(COLLATERAL_DENOM.to_string(), Uint128::zero());
    set_offer(&mut deps, &env, BORROWER, zeros, Timestamp::from_nanos(1)).unwrap();
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn increase_at_exactly_seven_days_is_stored() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = env
        .block
        .time
        .plus_seconds(DIRECTED_SALE_MAX_LIFETIME_SECONDS);
    set_offer(&mut deps, &env, BORROWER, seize(100), exp).expect("increase");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .expect("stored");
    assert_eq!(stored.amounts, seize(100));
    assert_eq!(stored.expires_at, exp);
}

#[test]
fn increase_one_second_past_seven_days_is_rejected() {
    let (mut deps, env) = setup_healthy_borrower();
    let too_far = env
        .block
        .time
        .plus_seconds(DIRECTED_SALE_MAX_LIFETIME_SECONDS + 1);
    let err = set_offer(&mut deps, &env, BORROWER, seize(100), too_far).unwrap_err();
    match err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("expires_at"), "message: {}", message);
            assert!(message.contains("7 days"), "message: {}", message);
        }
        other => panic!("expected IllegalArgumentError, got {:?}", other),
    }
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn increase_at_block_time_or_zero_fails_the_future_check() {
    let (mut deps, env) = setup_healthy_borrower();
    for expires_at in [env.block.time, Timestamp::from_nanos(0)] {
        let err = set_offer(&mut deps, &env, BORROWER, seize(100), expires_at).unwrap_err();
        match err {
            ContractError::IllegalArgumentError { message } => {
                assert_eq!(message, "expires_at must be in the future");
            }
            other => panic!("expected IllegalArgumentError, got {:?}", other),
        }
    }
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn legacy_offer_past_seven_days_keeps_deadline_on_decrease_then_increase_replaces_it() {
    let (mut deps, env) = setup_healthy_borrower();
    let legacy = env.block.time.plus_seconds(30 * 24 * 60 * 60);
    set_directed_sale_offer(
        deps.as_mut().storage,
        BORROWER,
        &DirectedSaleOfferV1 {
            amounts: seize(400),
            expires_at: legacy,
        },
    )
    .unwrap();

    adjust(
        &mut deps,
        &env,
        BORROWER,
        delta(-100),
        Timestamp::from_nanos(0),
    )
    .expect("decrease");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .expect("stored");
    assert_eq!(stored.amounts, seize(300));
    assert_eq!(stored.expires_at, legacy);

    let seven = env
        .block
        .time
        .plus_seconds(DIRECTED_SALE_MAX_LIFETIME_SECONDS);
    adjust(&mut deps, &env, BORROWER, delta(100), seven).expect("increase");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .expect("stored");
    assert_eq!(stored.amounts, seize(400));
    assert_eq!(stored.expires_at, seven);
}

#[test]
fn o6_borrower_with_collateral_and_no_debt_cannot_set() {
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
    set_oracle_prices(&mut deps.querier, prices_with_collateral("1.0"));
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(1000, COLLATERAL_DENOM)]),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();
    let err = set_offer(&mut deps, &env, BORROWER, seize(10), future(&env)).unwrap_err();
    illegal_contains(err, "no debt");
}

#[test]
fn o7_paused_blocks_set_and_liquidate() {
    let (mut deps, env) = setup_healthy_borrower();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::SetOperationalState {
            state: crate::model::OperationalState::Paused,
        },
    )
    .unwrap();
    let err = set_offer(&mut deps, &env, BORROWER, seize(10), future(&env)).unwrap_err();
    match err {
        ContractError::IllegalStateError { message } => {
            assert!(message.contains("paused"), "{}", message)
        }
        other => panic!("{:?}", other),
    }
    let err = liquidate(&mut deps, &env, OWNER, 450, seize(455)).unwrap_err();
    match err {
        ContractError::IllegalStateError { message } => {
            assert!(message.contains("paused"), "{}", message)
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn o8_frozen_allows_set_and_fill() {
    let (mut deps, env) = setup_healthy_borrower();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::SetOperationalState {
            state: crate::model::OperationalState::Frozen,
        },
    )
    .unwrap();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 450, seize(455)).expect("fill while frozen");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
}

#[test]
fn o9_query_returns_live_and_expired_offers() {
    let (mut deps, env) = setup_healthy_borrower();
    assert!(position(&deps, &env).directed_sale_offer.is_none());
    assert_eq!(position(&deps, &env).directed_sale_version, 0);
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(400), exp).unwrap();
    let got = position(&deps, &env).directed_sale_offer.unwrap();
    assert_eq!(got.amounts, seize(400));
    assert_eq!(got.expires_at, exp);

    let mut later = env.clone();
    later.block.time = later.block.time.plus_seconds(9_000);
    let got = position(&deps, &later).directed_sale_offer.unwrap();
    assert_eq!(got.expires_at, exp);

    set_offer(
        &mut deps,
        &env,
        BORROWER,
        BTreeMap::new(),
        Timestamp::from_nanos(0),
    )
    .unwrap();
    assert!(position(&deps, &env).directed_sale_offer.is_none());
}

#[test]
fn f1_healthy_exact_fill_clears_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 450, seize(455)).expect("directed fill");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    assert_eq!(
        get_borrower_collateral(deps.as_ref().storage, BORROWER)
            .unwrap()
            .amounts
            .get(COLLATERAL_DENOM)
            .copied(),
        Some(545)
    );
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        150
    );
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount, &coins(455, COLLATERAL_DENOM));
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn f2_unhealthy_exact_fill_succeeds() {
    let (mut deps, env) = setup_unhealthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 395, seize(455)).expect("unhealthy fill");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn f3_liquidatable_exact_fill_succeeds() {
    let (mut deps, env) = setup_liquidatable_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 374, seize(455)).expect("liquidatable fill");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
}

#[test]
fn f4_permissionless_directed_fill_skips_load_bearing_gate() {
    let (mut deps, env) = setup_healthy_borrower();
    set_liquidation_access(&mut deps, &env, LiquidationAccess::Permissionless);
    let mut prices = prices_with_collateral("1.0");
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(1000, UNRELIABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();
    let mut prices = prices_with_collateral("1.0");
    prices.insert(
        UNRELIABLE_COLLATERAL.to_string(),
        oracle_price_expired_for(
            Decimal256::from_str("1.0").unwrap(),
            env.block.time,
            DEFAULT_MAX_LIQUIDATION_STALENESS_SECONDS + 1,
        ),
    );
    set_oracle_prices(&mut deps.querier, prices);
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let res = liquidate(&mut deps, &env, OTHER, 450, seize(455)).expect("non-owner directed fill");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    assert_eq!(
        get_borrower_collateral(deps.as_ref().storage, BORROWER)
            .unwrap()
            .amounts
            .get(UNRELIABLE_COLLATERAL)
            .copied(),
        Some(1000),
        "unoffered unpriceable collateral stays"
    );
}

#[test]
fn f5_owner_only_rejects_non_owner() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let err = liquidate(&mut deps, &env, OTHER, 450, seize(455)).unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_LIQUIDATOR_ERR
    ));
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_some());
}

#[test]
fn f7_bonus_cap_still_rejects_directed_fill() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let err = liquidate(&mut deps, &env, OWNER, 400, seize(455)).unwrap_err();
    illegal_contains(err, "liquidation_bonus_rate");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(455)
    );
}

#[test]
fn f8_floor_still_rejects_when_remainder_is_valued() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(100), future(&env)).unwrap();
    let err = liquidate(&mut deps, &env, OWNER, 200, seize(100)).unwrap_err();
    illegal_contains(err, "below required 100%");
}

#[test]
fn f9_unpriceable_offered_asset_fails_seize() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    set_oracle_prices(&mut deps.querier, prices_with_collateral("0"));
    let err = liquidate(&mut deps, &env, OWNER, 450, seize(455)).unwrap_err();
    illegal_contains(err, "unpriceable");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_some());
}

#[test]
fn f10_full_repay_leaves_leftover_collateral() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(605), future(&env)).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 600, seize(605)).expect("full repay");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    assert!(res
        .attributes
        .iter()
        .all(|a| a.key != ATTRIBUTE_SWEPT_COLLATERAL_JSON));
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    assert_eq!(
        get_borrower_collateral(deps.as_ref().storage, BORROWER)
            .unwrap()
            .amounts
            .get(COLLATERAL_DENOM)
            .copied(),
        Some(395)
    );
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn f11_post_health_still_required() {
    let (mut deps, env) = setup_unhealthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(50), future(&env)).unwrap();
    let err = liquidate(&mut deps, &env, OWNER, 44, seize(50)).unwrap_err();
    illegal_contains(err, "margin_rate");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_some());
}

#[test]
fn f11_partial_nibble_on_unhealthy_still_fails_post_health() {
    let (mut deps, env) = setup_unhealthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(200), future(&env)).unwrap();
    let err = liquidate(&mut deps, &env, OWNER, 44, seize(50)).unwrap_err();
    illegal_contains(err, "margin_rate");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(200)
    );
}

#[test]
fn f12_excess_lending_funds_are_refunded() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(605), future(&env)).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 1000, seize(605)).expect("excess repay");
    assert_eq!(res.messages.len(), 2);
    match &res.messages[1].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount, &coins(400, LENDING_DENOM));
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn f13_partial_seize_on_healthy_book_is_directed() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(455), exp).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 99, seize(100)).expect("partial directed");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    assert_eq!(stored.amounts, seize(355));
    assert_eq!(stored.expires_at, exp);
    assert_eq!(
        get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap(),
        1
    );
}

#[test]
fn f13_partial_then_remainder_clears() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    liquidate(&mut deps, &env, OWNER, 99, seize(100)).expect("first partial");
    let res = liquidate(&mut deps, &env, OWNER, 349, seize(355)).expect("remainder");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    assert_eq!(
        get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap(),
        1
    );
}

#[test]
fn f13_multi_asset_subset_keeps_other_denom() {
    let (mut deps, env) = setup_healthy_borrower();
    let mut prices = prices_with_collateral("1.0");
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(100, UNRELIABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();
    let exp = future(&env);
    let mut offer = seize(100);
    offer.insert(UNRELIABLE_COLLATERAL.to_string(), Uint128::new(50));
    set_offer(&mut deps, &env, BORROWER, offer, exp).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 99, seize(100)).expect("btc subset");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.amounts.get(UNRELIABLE_COLLATERAL).copied(),
        Some(Uint128::new(50))
    );
    assert!(!stored.amounts.contains_key(COLLATERAL_DENOM));
    assert_eq!(stored.expires_at, exp);
}

#[test]
fn f14_bad_debt_sweeps_unoffered_remainder_and_permissionless_still_needs_owner() {
    let (mut deps, env) = setup_healthy_borrower();
    let mut prices = prices_with_collateral("1.0");
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(100, UNRELIABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();
    set_offer(&mut deps, &env, BORROWER, seize(1000), future(&env)).unwrap();
    let mut prices = prices_with_collateral("0.0005");
    prices.insert(
        UNRELIABLE_COLLATERAL.to_string(),
        oracle_price_expired_for(
            Decimal256::from_str("1.0").unwrap(),
            env.block.time,
            DEFAULT_MAX_LIQUIDATION_STALENESS_SECONDS + 1,
        ),
    );
    set_oracle_prices(&mut deps.querier, prices);

    set_liquidation_access(&mut deps, &env, LiquidationAccess::Permissionless);
    let err = liquidate(&mut deps, &env, OTHER, 1, seize(1000)).unwrap_err();
    assert!(
        matches!(
            &err,
            ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_UNPRICEABLE_ERR
        ),
        "{:?}",
        err
    );
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_some());

    let res = liquidate(&mut deps, &env, OWNER, 1, seize(1000)).expect("owner write-off fill");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    let swept = attr(&res, ATTRIBUTE_SWEPT_COLLATERAL_JSON);
    assert!(swept.contains(UNRELIABLE_COLLATERAL), "{}", swept);
    assert!(get_borrower_collateral(deps.as_ref().storage, BORROWER)
        .unwrap()
        .amounts
        .is_empty());
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn f15_empty_seize_is_not_a_directed_fill() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let err = liquidate(&mut deps, &env, OWNER, 450, BTreeMap::new()).unwrap_err();
    illegal_contains(err, "not liquidatable");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_some());
}

#[test]
fn c1_classic_liquidate_emits_directed_sale_false() {
    let (mut deps, env) = setup_liquidatable_borrower();
    let res = liquidate(&mut deps, &env, OWNER, 374, seize(455)).unwrap();
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "false");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn c3_non_exact_seize_on_liquidatable_book_keeps_the_offer() {
    let (mut deps, env) = setup_liquidatable_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(100), future(&env)).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 374, seize(455)).expect("classic");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "false");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(100)
    );
}

#[test]
fn c2_over_seize_on_healthy_book_fails_ltv_gate() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(100), future(&env)).unwrap();
    let err = liquidate(&mut deps, &env, OWNER, 450, seize(455)).unwrap_err();
    illegal_contains(err, "not liquidatable");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(100)
    );
}

#[test]
fn c2_extra_denom_on_healthy_book_fails_ltv_gate() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    let mut extra = seize(100);
    extra.insert(UNRELIABLE_COLLATERAL.to_string(), Uint128::new(1));
    let err = liquidate(&mut deps, &env, OWNER, 99, extra).unwrap_err();
    illegal_contains(err, "not liquidatable");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(455)
    );
}

#[test]
fn c3_partial_does_not_block_later_classic_liquidation() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    liquidate(&mut deps, &env, OWNER, 99, seize(100)).expect("partial directed");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(355)
    );
    set_oracle_prices(&mut deps.querier, prices_with_collateral("0.75"));
    let res = liquidate(&mut deps, &env, OWNER, 340, seize(455)).expect("classic over remaining");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "false");
    // Remaining debt 161 at index 1, price 0.75. Cap is 161 * 1.02. Listing USD is
    // 355 * 0.75. floor(355 * 164.22 / 266.25) = 218. Balance left is 445, so the
    // clamp does not reduce 355 before that floor.
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .borrow_index,
        Decimal256::one()
    );
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(218)
    );
}

#[test]
fn c4_expired_exact_seize_on_healthy_book_fails() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(455), exp).unwrap();
    let mut later = env.clone();
    later.block.time = exp;
    let err = execute(
        deps.as_mut(),
        later,
        message_info(&Addr::unchecked(OWNER), &[coin(450, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize(455),
            min_collateral_out: None,
        },
    )
    .unwrap_err();
    illegal_contains(err, "not liquidatable");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_some());
}

#[test]
fn c5_expired_offer_does_not_block_classic_liquidation() {
    let (mut deps, env) = setup_liquidatable_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(455), exp).unwrap();
    let mut later = env.clone();
    later.block.time = exp.plus_seconds(1);
    let res = execute(
        deps.as_mut(),
        later,
        message_info(&Addr::unchecked(OWNER), &[coin(374, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: seize(455),
            min_collateral_out: None,
        },
    )
    .unwrap();
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "false");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    assert_eq!(stored.expires_at, exp);
    // 3601 seconds later the index has accrued, so a 374 repay removes 373 scaled
    // units and leaves 227. Ceil payoff is 228. At price 0.83 the listing is worth
    // 377.65, the cap is 228 * 1.02, and floor(455 * 232.56 / 377.65) = 280.
    // Collateral left is 545, so the clamp does not reduce 455 before that floor.
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        227
    );
    assert_eq!(stored.amounts, seize(280));
}

#[test]
fn c6_permissionless_classic_load_bearing_still_requires_owner() {
    let (mut deps, env) = setup_liquidatable_borrower();
    set_liquidation_access(&mut deps, &env, LiquidationAccess::Permissionless);
    let mut prices = prices_with_collateral("0.83");
    prices.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(1000, UNRELIABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();
    let mut prices = prices_with_collateral("0.83");
    prices.insert(
        UNRELIABLE_COLLATERAL.to_string(),
        oracle_price_expired_for(
            Decimal256::from_str("1.0").unwrap(),
            env.block.time,
            DEFAULT_MAX_LIQUIDATION_STALENESS_SECONDS + 1,
        ),
    );
    set_oracle_prices(&mut deps.querier, prices);
    let err = liquidate(&mut deps, &env, OTHER, 374, seize(455)).unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_UNPRICEABLE_ERR
    ));
}

#[test]
fn i1_remove_can_leave_offer_larger_than_balance() {
    let (mut deps, env) = setup_borrower(100, "1.0");
    set_offer(&mut deps, &env, BORROWER, seize(800), future(&env)).unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral {
            to_remove: seize(300),
        },
    )
    .expect("remove stays healthy");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(800)
    );
    let err = liquidate(&mut deps, &env, OWNER, 100, seize(800)).unwrap_err();
    illegal_contains(err, "insufficient collateral");
}

#[test]
fn full_repay_clears_the_offer_and_leaves_the_version() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(100), future(&env)).unwrap();
    let version = get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap();
    repay(&mut deps, &env, 600).unwrap();
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    assert_eq!(
        get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap(),
        version
    );
    assert!(position(&deps, &env).directed_sale_offer.is_none());
    let err = liquidate(&mut deps, &env, OWNER, 1, seize(100)).unwrap_err();
    illegal_contains(err, "no debt");
}

#[test]
fn i3_add_collateral_leaves_the_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(400), future(&env)).unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(50, COLLATERAL_DENOM)]),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(400)
    );
}

#[test]
fn version_must_be_stored_plus_one() {
    let (mut deps, env) = setup_healthy_borrower();
    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::AdjustDirectedCollateralSale {
            adjustments: delta(100),
            expires_at: future(&env),
            version: 2,
        },
    )
    .unwrap_err();
    illegal_contains(err, "version must be 1");

    set_offer(&mut deps, &env, BORROWER, seize(100), future(&env)).unwrap();
    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::AdjustDirectedCollateralSale {
            adjustments: delta(10),
            expires_at: future(&env),
            version: 1,
        },
    )
    .unwrap_err();
    illegal_contains(err, "version must be 2");
}

#[test]
fn fill_does_not_reset_version_and_increase_after_fill_is_residual() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    liquidate(&mut deps, &env, OWNER, 450, seize(455)).expect("fill 455");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    assert_eq!(
        get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap(),
        1
    );

    let res = adjust(&mut deps, &env, BORROWER, delta(10), future(&env)).expect("+10 after fill");
    assert_eq!(attr(&res, ATTRIBUTE_VERSION), "2");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(10)
    );
    liquidate(&mut deps, &env, OWNER, 10, seize(10)).expect("fill residual 10");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    assert_eq!(
        get_borrower_collateral(deps.as_ref().storage, BORROWER)
            .unwrap()
            .amounts
            .get(COLLATERAL_DENOM)
            .copied(),
        Some(535)
    );
}

#[test]
fn decrease_after_fill_clamps_and_succeeds() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(455), future(&env)).unwrap();
    liquidate(&mut deps, &env, OWNER, 450, seize(455)).unwrap();
    let res = adjust(&mut deps, &env, BORROWER, delta(-50), future(&env)).expect("clamp -50");
    assert_eq!(attr(&res, ATTRIBUTE_APPLIED_JSON), "{}");
    assert_eq!(attr(&res, ATTRIBUTE_VERSION), "2");
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn multi_asset_decrease_still_applies_when_one_asset_already_zero() {
    let (mut deps, env) = setup_healthy_borrower();
    set_oracle_prices(&mut deps.querier, {
        let mut p = prices_with_collateral("1.0");
        p.insert(UNRELIABLE_COLLATERAL.to_string(), price_entry("1.0"));
        p
    });
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(1000, UNRELIABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();

    let mut open = BTreeMap::new();
    open.insert(COLLATERAL_DENOM.to_string(), Int128::from(50i128));
    open.insert(UNRELIABLE_COLLATERAL.to_string(), Int128::from(20i128));
    adjust(&mut deps, &env, BORROWER, open, future(&env)).unwrap();

    adjust(&mut deps, &env, BORROWER, delta(-50), future(&env)).unwrap();
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.amounts.get(UNRELIABLE_COLLATERAL).copied(),
        Some(Uint128::new(20))
    );
    assert!(!stored.amounts.contains_key(COLLATERAL_DENOM));

    let mut close = BTreeMap::new();
    close.insert(COLLATERAL_DENOM.to_string(), Int128::from(-50i128));
    close.insert(UNRELIABLE_COLLATERAL.to_string(), Int128::from(-20i128));
    let res = adjust(&mut deps, &env, BORROWER, close, future(&env)).expect("clamp A, clear B");
    assert!(attr(&res, ATTRIBUTE_APPLIED_JSON).contains(UNRELIABLE_COLLATERAL));
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn decrease_after_full_repay_does_not_require_debt() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(100), exp).unwrap();
    repay(&mut deps, &env, 600).unwrap();
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    // The repay deleted the row. Put a listing back so a decrease can still run
    // when scaled debt is already zero.
    set_directed_sale_offer(
        deps.as_mut().storage,
        BORROWER,
        &DirectedSaleOfferV1 {
            amounts: seize(100),
            expires_at: exp,
        },
    )
    .unwrap();
    adjust(&mut deps, &env, BORROWER, delta(-40), future(&env)).expect("decrease with no debt");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(60)
    );
    let err = adjust(&mut deps, &env, BORROWER, delta(10), future(&env)).unwrap_err();
    illegal_contains(err, "no debt");
}

#[test]
fn partial_repay_below_the_cap_keeps_the_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(100), exp).unwrap();
    let version = get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap();
    repay(&mut deps, &env, 1).unwrap();
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .borrow_index,
        Decimal256::one()
    );
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        599
    );
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    assert_eq!(stored.amounts, seize(100));
    assert_eq!(stored.expires_at, exp);
    assert_eq!(
        get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap(),
        version
    );
}

#[test]
fn partial_repay_above_the_cap_floors_the_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(1000), exp).unwrap();
    repay(&mut deps, &env, 1).unwrap();
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .borrow_index,
        Decimal256::one()
    );
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    // floor(1000 * (599 * 1.02) / 1000) = 610. Collateral balance is 1000, so the clamp is a no-op.
    assert_eq!(stored.amounts, seize(610));
    assert_eq!(stored.expires_at, exp);
    assert_eq!(
        get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap(),
        1
    );
}

#[test]
fn repay_with_a_stale_collateral_price_keeps_the_clamped_offer() {
    let (mut deps, env) = setup_borrower(100, "1.0");
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(800), exp).unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral {
            to_remove: seize(300),
        },
    )
    .expect("remove stays healthy");
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(
        COLLATERAL_DENOM.to_string(),
        stale_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps.querier, prices);
    repay(&mut deps, &env, 1).unwrap();
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        99
    );
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    // Balance is 700. The stale collateral price skips the dollar cap, which would
    // otherwise floor 700 down to the ~101 payoff cap.
    assert_eq!(stored.amounts, seize(700));
    assert_eq!(stored.expires_at, exp);
}

#[test]
fn borrow_after_full_repay_leaves_no_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(100), exp).unwrap();
    repay(&mut deps, &env, 600).unwrap();
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    set_directed_sale_offer(
        deps.as_mut().storage,
        BORROWER,
        &DirectedSaleOfferV1 {
            amounts: seize(100),
            expires_at: exp,
        },
    )
    .unwrap();
    borrow_more(&mut deps, &env, 20).unwrap();
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        20
    );
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
    let err = liquidate(&mut deps, &env, OWNER, 20, seize(100)).unwrap_err();
    illegal_contains(err, "not liquidatable");
    assert_eq!(
        get_borrower_collateral(deps.as_ref().storage, BORROWER)
            .unwrap()
            .amounts
            .get(COLLATERAL_DENOM)
            .copied(),
        Some(1000)
    );
}

#[test]
fn larger_borrow_keeps_a_live_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(100), exp).unwrap();
    borrow_more(&mut deps, &env, 20).unwrap();
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        620
    );
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    assert_eq!(stored.amounts, seize(100));
    assert_eq!(stored.expires_at, exp);
}

#[test]
fn borrow_deletes_an_expired_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    set_directed_sale_offer(
        deps.as_mut().storage,
        BORROWER,
        &DirectedSaleOfferV1 {
            amounts: seize(100),
            expires_at: env.block.time,
        },
    )
    .unwrap();
    borrow_more(&mut deps, &env, 20).unwrap();
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        620
    );
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn rejected_borrow_keeps_an_expired_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = env.block.time;
    set_directed_sale_offer(
        deps.as_mut().storage,
        BORROWER,
        &DirectedSaleOfferV1 {
            amounts: seize(100),
            expires_at: exp,
        },
    )
    .unwrap();
    let err = borrow_more(&mut deps, &env, 500).unwrap_err();
    illegal_contains(err, "Loan-to-value");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    assert_eq!(stored.amounts, seize(100));
    assert_eq!(stored.expires_at, exp);
}

#[test]
fn directed_fill_of_a_subset_applies_the_cap_to_what_remains() {
    let (mut deps, env) = setup_healthy_borrower();
    let exp = future(&env);
    set_offer(&mut deps, &env, BORROWER, seize(1000), exp).unwrap();
    let res = liquidate(&mut deps, &env, OWNER, 99, seize(100)).expect("subset");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    let stored = get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .unwrap();
    // Seize 100 leaves 900. Debt payoff is 501. floor(900 * (501 * 1.02) / 900) = 511.
    assert_eq!(stored.amounts, seize(511));
    assert_eq!(stored.expires_at, exp);
    assert_eq!(
        get_directed_sale_version(deps.as_ref().storage, BORROWER).unwrap(),
        1
    );
}

#[test]
fn fit_price_rise_seizes_306_clears_debt_and_refunds_nothing() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(612), future(&env)).unwrap();
    set_oracle_prices(&mut deps.querier, prices_with_collateral("2.0"));
    // 612 units at $2 are worth $1224. Repay stays 600, cap is 612.
    // floor(612 * 612 / 1224) = 306.
    let res = liquidate_fit(
        &mut deps,
        &env,
        600,
        seize(612),
        mins(&[(COLLATERAL_DENOM, 300)]),
    )
    .expect("price rise fit");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "true");
    assert_eq!(attr(&res, ATTRIBUTE_AMOUNT), "600");
    assert_eq!(bank_amounts(&res), vec![coins(306, COLLATERAL_DENOM)]);
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    assert!(get_directed_sale_offer(deps.as_ref().storage, BORROWER)
        .unwrap()
        .is_none());
}

#[test]
fn fit_price_drop_repays_550_refunds_50_and_seizes_the_signed_amount() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(612), future(&env)).unwrap();
    set_oracle_prices(&mut deps.querier, prices_with_collateral("0.90"));
    let res = liquidate_fit(
        &mut deps,
        &env,
        600,
        seize(612),
        mins(&[(COLLATERAL_DENOM, 300)]),
    )
    .expect("price drop fit");
    assert_eq!(attr(&res, ATTRIBUTE_AMOUNT), "550");
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        50
    );
    assert_eq!(
        bank_amounts(&res),
        vec![coins(612, COLLATERAL_DENOM), coins(50, LENDING_DENOM)]
    );
}

#[test]
fn fit_below_min_collateral_out_reverts_and_keeps_the_offer() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(612), future(&env)).unwrap();
    set_oracle_prices(&mut deps.querier, prices_with_collateral("3"));
    // floor(612 * 612 / 1836) = 204, below the floor of 250.
    let err = liquidate_fit(
        &mut deps,
        &env,
        600,
        seize(612),
        mins(&[(COLLATERAL_DENOM, 250)]),
    )
    .unwrap_err();
    illegal_contains(err, "min_collateral_out");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(612)
    );
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        600
    );
}

#[test]
fn fit_two_assets_share_one_ratio() {
    let (mut deps, env) = setup_two_asset_borrower();
    let offer = two_asset_amounts(400);
    set_offer(&mut deps, &env, BORROWER, offer.clone(), future(&env)).unwrap();
    // Values $400 and $600, cap $612. Both amounts are 400.
    // floor(400 * 612 / 1000) = 244.
    let res = liquidate_fit(
        &mut deps,
        &env,
        600,
        offer,
        mins(&[(COLLATERAL_DENOM, 200), (UNRELIABLE_COLLATERAL, 200)]),
    )
    .expect("shared ratio");
    assert_eq!(
        bank_amounts(&res),
        vec![vec![
            coin(244, COLLATERAL_DENOM),
            coin(244, UNRELIABLE_COLLATERAL),
        ]]
    );
}

#[test]
fn fit_one_asset_below_its_floor_reverts_the_whole_fill() {
    let (mut deps, env) = setup_two_asset_borrower();
    let offer = two_asset_amounts(400);
    set_offer(&mut deps, &env, BORROWER, offer.clone(), future(&env)).unwrap();
    let err = liquidate_fit(
        &mut deps,
        &env,
        600,
        offer,
        mins(&[(COLLATERAL_DENOM, 200), (UNRELIABLE_COLLATERAL, 245)]),
    )
    .unwrap_err();
    illegal_contains(err, "min_collateral_out");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        two_asset_amounts(400)
    );
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        600
    );
}

#[test]
fn fit_lowered_repayment_that_would_be_bad_debt_reverts() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(1000), future(&env)).unwrap();
    set_oracle_prices(&mut deps.querier, prices_with_collateral("0.5"));
    let err = liquidate_fit(
        &mut deps,
        &env,
        600,
        seize(1000),
        mins(&[(COLLATERAL_DENOM, 1000)]),
    )
    .unwrap_err();
    illegal_contains(err, "bad debt");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(1000)
    );
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        600
    );
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .deficit_underlying,
        0
    );
}

#[test]
fn liquidate_without_min_collateral_out_rejects_an_oversized_seize() {
    let (mut deps, env) = setup_healthy_borrower();
    set_offer(&mut deps, &env, BORROWER, seize(612), future(&env)).unwrap();
    set_oracle_prices(&mut deps.querier, prices_with_collateral("2.0"));
    let err = liquidate(&mut deps, &env, OWNER, 600, seize(612)).unwrap_err();
    illegal_contains(err, "exceeds allowed maximum");
    assert_eq!(
        get_directed_sale_offer(deps.as_ref().storage, BORROWER)
            .unwrap()
            .unwrap()
            .amounts,
        seize(612)
    );
}

#[test]
fn classic_liquidation_with_min_collateral_out_scales_on_a_price_rise() {
    let (mut deps, env) = setup_borrower(600, "0.80");
    set_oracle_prices(&mut deps.querier, prices_with_collateral("0.83"));
    // 765 is the bonus-cap seize at $0.80 (612 / 0.80). At $0.83 that basket
    // is worth $634.95, so floor(765 * 612 / 634.95) = 737.
    let res = liquidate_fit(
        &mut deps,
        &env,
        600,
        seize(765),
        mins(&[(COLLATERAL_DENOM, 700)]),
    )
    .expect("classic fit");
    assert_eq!(attr(&res, ATTRIBUTE_DIRECTED_SALE), "false");
    assert_eq!(attr(&res, ATTRIBUTE_AMOUNT), "600");
    assert_eq!(bank_amounts(&res), vec![coins(737, COLLATERAL_DENOM)]);
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
}
