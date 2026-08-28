//! Tests for WriteOff: owner-only dust/zero-price close that books residual debt.

use crate::constants::{
    ATTRIBUTE_BAD_DEBT_UNDERLYING, ATTRIBUTE_COLLATERAL_JSON, ATTRIBUTE_DEFICIT_UNDERLYING,
    ATTRIBUTE_SCALED_AMOUNT, ATTRIBUTE_UNPRICEABLE_JSON,
};
use crate::contract::execute;
use crate::execute::write_off::{ACTION, ASSERT_OWNER_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{
    BadDebtLossAllocation, CollateralAssetV1, Denom, OperationalState, RateParamsV1,
};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{
    get_borrower_collateral, get_contract_state_v1, get_reserve_state_v1, get_scaled_borrow,
    set_contract_state_v1,
};
use crate::tests::query::common::{CUSTODIAN, OWNER};
use crate::tests::reserve_invariant::assert_reserve_assets_liabilities_tie_out;
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, BankMsg, ContractResult, CosmosMsg, Decimal256, Env,
    MemoryStorage, OwnedDeps, QuerierResult, SystemError, SystemResult, Uint128, WasmQuery,
};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use std::collections::HashMap;
use std::str::FromStr;

const BORROWER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";
const OTHER: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";
const LENDING_DENOM: &str = "uylds.fcc";
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
const COLLATERAL_DENOM: &str = "nbtc.figure.se";
const UNPRICEABLE_COLLATERAL: &str = "neth.figure.se";

fn default_instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        contract_name: "pool-v2-write-off".to_string(),
        description: "WriteOff tests".to_string(),
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
        supported_collateral_assets: vec![
            CollateralAssetV1 {
                asset_id: COLLATERAL_DENOM.to_string(),
                haircut: None,
            },
            CollateralAssetV1 {
                asset_id: UNPRICEABLE_COLLATERAL.to_string(),
                haircut: None,
            },
        ],
        commit_market_id: None,
        bad_debt_loss_allocation: Default::default(),
        custodian: CUSTODIAN.to_owned(),
        max_liquidation_staleness_seconds: 86400,
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

/// Borrow 700 against 1000 collateral at price 1, then crash collateral to `crash_price`.
fn setup_borrower_after_price_crash(
    crash_price: &str,
) -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
) {
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

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

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

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(700),
        },
    )
    .expect("borrow");

    prices.insert(COLLATERAL_DENOM.to_string(), price_entry(crash_price));
    set_oracle_prices(&mut deps.querier, prices);
    (deps, env)
}

#[test]
fn write_off_non_owner_fails() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OTHER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn write_off_fails_when_paused() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let mut c = get_contract_state_v1(deps.as_ref().storage).unwrap();
    c.operational_state = OperationalState::Paused;
    set_contract_state_v1(deps.as_mut().storage, &c).unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(message.contains("paused"), "message: {}", message);
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

#[test]
fn write_off_rejects_collateral_worth_at_least_one_lending_unit() {
    let (mut deps, env) = setup_borrower_after_price_crash("0.65");
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("use Liquidate"), "message: {}", message);
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn write_off_zero_price_books_deficit_and_leaves_unpriceable_collateral() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off");

    assert_eq!(res.attributes[0].value, ACTION);
    let bad_debt = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_BAD_DEBT_UNDERLYING)
        .expect("bad_debt_underlying");
    assert_eq!(bad_debt.value, "700");
    let deficit_attr = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_DEFICIT_UNDERLYING)
        .expect("deficit_underlying");
    assert_eq!(deficit_attr.value, "700");
    let scaled = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_SCALED_AMOUNT)
        .expect("scaled_amount");
    assert_eq!(scaled.value, "0");
    let collateral_json = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_COLLATERAL_JSON)
        .expect("collateral_json");
    assert_eq!(collateral_json.value, "{}");
    let unpriceable = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_UNPRICEABLE_JSON)
        .expect("unpriceable_json");
    let ids: Vec<String> = serde_json::from_str(&unpriceable.value).unwrap();
    assert_eq!(ids, vec![COLLATERAL_DENOM.to_string()]);

    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 700);
    let leftover = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(*leftover.amounts.get(COLLATERAL_DENOM).unwrap(), 1000u128);
    assert!(res.messages.is_empty());

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert_reserve_assets_liabilities_tie_out(deps.as_ref().storage, "after zero-price write_off")
        .unwrap();
}

#[test]
fn write_off_sub_one_unit_collateral_books_deficit() {
    // 1000 * 0.0005 = 0.5 USD < 1 lending unit
    let (mut deps, env) = setup_borrower_after_price_crash("0.0005");
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off dust");

    let bad_debt = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_BAD_DEBT_UNDERLYING)
        .expect("bad_debt_underlying");
    assert_eq!(bad_debt.value, "700");
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .deficit_underlying,
        700
    );
    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount[0].denom, COLLATERAL_DENOM);
            assert_eq!(amount[0].amount, Uint128::new(1000));
        }
        _ => panic!("expected priced-dust BankMsg::Send"),
    }
    assert!(get_borrower_collateral(deps.as_ref().storage, BORROWER)
        .unwrap()
        .amounts
        .is_empty());
    assert_reserve_assets_liabilities_tie_out(deps.as_ref().storage, "after dust write_off")
        .unwrap();
}

#[test]
fn write_off_applies_optional_repay_then_books_residual() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(50, LENDING_DENOM)]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off with repay");

    let bad_debt = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_BAD_DEBT_UNDERLYING)
        .expect("bad_debt_underlying");
    assert_eq!(bad_debt.value, "650");
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .deficit_underlying,
        650
    );
}

#[test]
fn write_off_full_repay_covers_deficit() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(700, LENDING_DENOM)]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off full repay");

    assert!(res
        .attributes
        .iter()
        .all(|a| a.key != ATTRIBUTE_BAD_DEBT_UNDERLYING));
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .deficit_underlying,
        0
    );
    assert!(res.messages.is_empty());
    let leftover = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(*leftover.amounts.get(COLLATERAL_DENOM).unwrap(), 1000u128);
    assert_reserve_assets_liabilities_tie_out(deps.as_ref().storage, "after full-repay write_off")
        .unwrap();
}

#[test]
fn write_off_refunds_excess_lending() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(800, LENDING_DENOM)]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off excess");

    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .deficit_underlying,
        0
    );
    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, LENDING_DENOM);
            assert_eq!(amount[0].amount, Uint128::new(100));
        }
        _ => panic!("expected excess lending refund"),
    }
}

#[test]
fn write_off_succeeds_when_frozen() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let mut c = get_contract_state_v1(deps.as_ref().storage).unwrap();
    c.operational_state = OperationalState::Frozen;
    set_contract_state_v1(deps.as_mut().storage, &c).unwrap();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off when frozen");
    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
}

#[test]
fn write_off_custodian_fails() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn write_off_rejects_collateral_worth_exactly_one_lending_unit() {
    // 1000 * 0.001 = 1.0 USD == one lending unit at price 1
    let (mut deps, env) = setup_borrower_after_price_crash("0.001");
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("use Liquidate"), "message: {}", message);
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn write_off_immediate_haircut_skips_deficit() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let mut msg = default_instantiate_msg();
    msg.bad_debt_loss_allocation = BadDebtLossAllocation::ImmediateLiquidityIndexHaircut;
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate");

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

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
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(700),
        },
    )
    .expect("borrow");
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0"));
    set_oracle_prices(&mut deps.querier, prices);

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off immediate");

    let deficit_attr = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_DEFICIT_UNDERLYING)
        .expect("deficit_underlying");
    assert_eq!(deficit_attr.value, "0");
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .deficit_underlying,
        0
    );
}

#[test]
fn liquidate_still_fails_band_when_collateral_price_is_zero() {
    let (mut deps, env) = setup_borrower_after_price_crash("0");
    let mut all = std::collections::BTreeMap::new();
    all.insert(COLLATERAL_DENOM.to_string(), Uint128::new(1000));
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::Liquidate {
            borrower: BORROWER.to_string(),
            collateral_to_seize: all,
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("No priceable collateral")
                    || message.contains("unpriceable")
                    || message.contains("below required")
                    || message.contains("100%"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError band failure, got {:?}", err),
    }
}

#[test]
fn write_off_fails_when_no_debt() {
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
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("no debt"), "message: {}", message);
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn write_off_succeeds_when_collateral_has_no_stored_price() {
    let (mut deps, env) = setup_borrower_after_price_crash("1.0");
    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices);

    let res = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off with missing collateral feed");

    assert_eq!(
        get_scaled_borrow(deps.as_ref().storage, BORROWER).unwrap(),
        0
    );
    assert_eq!(
        get_reserve_state_v1(deps.as_ref().storage)
            .unwrap()
            .deficit_underlying,
        700
    );
    let bad_debt = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_BAD_DEBT_UNDERLYING)
        .expect("bad_debt_underlying");
    assert_eq!(bad_debt.value, "700");
    let leftover = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(*leftover.amounts.get(COLLATERAL_DENOM).unwrap(), 1000u128);
    assert!(res.messages.is_empty());
    let unpriceable = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_UNPRICEABLE_JSON)
        .expect("unpriceable_json");
    let ids: Vec<String> = serde_json::from_str(&unpriceable.value).unwrap();
    assert_eq!(ids, vec![COLLATERAL_DENOM.to_string()]);

    let mut to_remove = std::collections::BTreeMap::new();
    to_remove.insert(COLLATERAL_DENOM.to_string(), Uint128::new(1000));
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .expect("borrower reclaims unpriceable collateral after debt is written off");
    assert!(get_borrower_collateral(deps.as_ref().storage, BORROWER)
        .unwrap()
        .amounts
        .is_empty());
}

#[test]
fn write_off_seizes_priced_dust_and_leaves_unpriceable_collateral() {
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

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    prices.insert(UNPRICEABLE_COLLATERAL.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

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
    .expect("add priced");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(5_000, UNPRICEABLE_COLLATERAL)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add later-unpriceable");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(700),
        },
    )
    .expect("borrow");

    // Priced dust (1000 * 0.0005 = $0.50) plus a missing feed on the large bag.
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("0.0005"));
    prices.remove(UNPRICEABLE_COLLATERAL);
    set_oracle_prices(&mut deps.querier, prices);

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .expect("write_off mixed bag");

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, COLLATERAL_DENOM);
            assert_eq!(amount[0].amount, Uint128::new(1000));
        }
        _ => panic!("expected priced-dust BankMsg::Send"),
    }
    let leftover = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(
        *leftover.amounts.get(UNPRICEABLE_COLLATERAL).unwrap(),
        5_000u128
    );
    assert!(!leftover.amounts.contains_key(COLLATERAL_DENOM));
    let unpriceable = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_UNPRICEABLE_JSON)
        .expect("unpriceable_json");
    let ids: Vec<String> = serde_json::from_str(&unpriceable.value).unwrap();
    assert_eq!(ids, vec![UNPRICEABLE_COLLATERAL.to_string()]);
}

#[test]
fn write_off_rejects_high_precision_cheap_display_worth_at_least_one_unit() {
    // 3 whole 18-decimal tokens at $0.50 display = $1.50. Deprecated scaled price_usd
    // floors to 0, which would have classified this bag as dust.
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

    let mut prices = HashMap::new();
    prices.insert(LENDING_DENOM.to_string(), price_entry("1.0"));
    prices.insert(COLLATERAL_DENOM.to_string(), price_entry("1.0"));
    set_oracle_prices(&mut deps.querier, prices.clone());

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(1000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    let amount = 3_000_000_000_000_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(BORROWER),
            &[coin(amount, COLLATERAL_DENOM)],
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

    #[allow(deprecated)]
    let cheap = AssetPriceResponseV1 {
        price_usd: Decimal256::zero(),
        display_price_usd: Decimal256::from_str("0.5").unwrap(),
        precision: 18,
        as_of_epoch_second: 0,
        expiration_epoch_seconds: u64::MAX,
    };
    prices.insert(COLLATERAL_DENOM.to_string(), cheap);
    set_oracle_prices(&mut deps.querier, prices);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WriteOff {
            borrower: BORROWER.to_string(),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("use Liquidate"), "message: {}", message);
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
