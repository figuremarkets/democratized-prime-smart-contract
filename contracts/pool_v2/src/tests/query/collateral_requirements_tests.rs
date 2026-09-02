//! Tests for GetCollateralRequirements query.

use crate::contract::query;
use crate::model::error::{ContractError, QueryError};
use crate::model::{BorrowerCollateralV1, CollateralRequirementsResponseV1, ReserveStateV1};
use crate::msg::QueryMsg;
use crate::storage::{
    get_reserve_state_v1, set_borrower_collateral, set_reserve_state_v1, set_scaled_borrow,
};
use crate::tests::fixtures::{fresh_oracle_price, stale_oracle_price};
use crate::tests::query::common::{setup_instantiated, ORACLE, SOME_USER};
use cosmwasm_std::testing::MockApi;
use cosmwasm_std::{
    from_json, to_json_binary, ContractResult, MemoryStorage, OwnedDeps, QuerierResult,
    SystemError, SystemResult, WasmQuery,
};
use cosmwasm_std::{Decimal256, Uint128};
use democratized_prime_lib::price_oracle::model::AssetPriceResponseV1;
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use std::collections::{BTreeMap, HashMap};

fn set_oracle_prices(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    prices: HashMap<String, AssetPriceResponseV1>,
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
    deps.querier.mock_querier.update_wasm(handler);
}

/// No borrower and zero new loan → short-circuit returns zeros (no oracle call).
#[test]
fn get_collateral_requirements_zero_loan_no_borrower_returns_zeros() {
    let (deps, env) = setup_instantiated();
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: None,
            new_loan_amount: Uint128::zero(),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetCollateralRequirements response");
    assert_eq!(res["required_collateral_value_usd"], "0");
    assert_eq!(res["additional_collateral_value_usd"], "0");
    let required = res["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0]["asset_id"], "asset.one");
    assert_eq!(required[0]["amount"], "0");
    assert_eq!(required[0]["satisfiable"], true);
}

/// Borrower with no existing debt and zero new loan → full path returns zeros.
#[test]
fn get_collateral_requirements_zero_loan_borrower_no_debt_returns_zeros() {
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: Some(SOME_USER.to_string()),
            new_loan_amount: Uint128::zero(),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetCollateralRequirements response");
    assert_eq!(res["required_collateral_value_usd"], "0");
    assert_eq!(res["additional_collateral_value_usd"], "0");
    let required = res["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0]["asset_id"], "asset.one");
    assert_eq!(required[0]["amount"], "0");
    assert_eq!(required[0]["satisfiable"], true);
}

#[test]
fn get_collateral_requirements_fails_when_oracle_has_no_lending_denom_price() {
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "asset.one".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    let err = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: None,
            new_loan_amount: Uint128::new(1000),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .unwrap_err();
    match &err {
        QueryError::Contract(_) => {}
        _ => panic!(
            "expected Contract error (missing lending denom price), got {:?}",
            err
        ),
    }
    let msg = err.to_string();
    assert!(
        msg.contains("Price of asset") && msg.contains("uylds.fcc"),
        "error should mention missing price for lending denom: {}",
        msg
    );
}

#[test]
fn get_collateral_requirements_returns_zero_amount_when_requested_asset_has_no_price() {
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: None,
            new_loan_amount: Uint128::new(1000),
            collateral_assets: vec!["asset.one".to_string(), "asset.unknown".to_string()],
        },
    )
    .expect("missing requested asset should be valued at zero, not error");
    let resp: CollateralRequirementsResponseV1 = from_json(bin).unwrap();
    let unknown = resp
        .required
        .iter()
        .find(|r| r.asset_id == "asset.unknown")
        .expect("preserve 1:1 with collateral_assets");
    assert!(unknown.amount.is_zero());
    assert!(!unknown.satisfiable);
    let priced = resp
        .required
        .iter()
        .find(|r| r.asset_id == "asset.one")
        .expect("priced requested asset stays quotable");
    assert!(priced.satisfiable);
    assert!(!priced.amount.is_zero());
}

#[test]
fn get_collateral_requirements_with_oracle_returns_required_value_and_per_asset() {
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: None,
            new_loan_amount: Uint128::new(1000),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetCollateralRequirements response");
    let required_val: &str = res["required_collateral_value_usd"].as_str().unwrap();
    assert!(
        required_val.starts_with("1250"),
        "required_collateral_value_usd {}",
        required_val
    );
    let required = res["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0]["asset_id"], "asset.one");
    assert_eq!(required[0]["amount"], "1563");
    assert_eq!(required[0]["satisfiable"], true);
}

/// When borrower has existing collateral, per-asset "required" is the *additional* amount needed, not the full amount.
#[test]
fn get_collateral_requirements_with_borrower_subtracts_existing_collateral() {
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    // Borrower already has 500 asset.one. At price 1 and 80% haircut, value = 400 USD.
    let mut amounts = BTreeMap::new();
    amounts.insert("asset.one".to_string(), 500u128);
    let collateral = BorrowerCollateralV1 { amounts };
    set_borrower_collateral(deps.as_mut().storage, SOME_USER, &collateral).expect("set collateral");

    // New loan 1000 → required total = 1000/0.8 = 1250 USD. Existing = 400 → need 850 more.
    // Per asset.one: 850 / 0.8 = 1062.5 → ceil 1063.
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: Some(SOME_USER.to_string()),
            new_loan_amount: Uint128::new(1000),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetCollateralRequirements response");
    let required_val: &str = res["required_collateral_value_usd"].as_str().unwrap();
    assert!(
        required_val.starts_with("1250"),
        "required_collateral_value_usd {}",
        required_val
    );
    let additional_val: &str = res["additional_collateral_value_usd"].as_str().unwrap();
    assert!(
        additional_val.starts_with("850"),
        "additional_collateral_value_usd {}",
        additional_val
    );
    let required = res["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0]["asset_id"], "asset.one");
    assert_eq!(
        required[0]["amount"], "1063",
        "additional asset.one needed (850/0.8 ceil), not full 1563"
    );
    assert_eq!(required[0]["satisfiable"], true);
}

/// When an asset has value_per_unit zero (e.g. oracle price 0), it is still included in `required`
/// with amount "0", preserving 1:1 mapping between input collateral_assets and output required.
#[test]
fn get_collateral_requirements_zero_value_per_unit_includes_asset_with_zero_amount() {
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        fresh_oracle_price(Decimal256::zero(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: None,
            new_loan_amount: Uint128::new(1000),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetCollateralRequirements response");
    let required = res["required"].as_array().unwrap();
    assert_eq!(
        required.len(),
        1,
        "required must have one entry per requested asset"
    );
    assert_eq!(required[0]["asset_id"], "asset.one");
    assert_eq!(
        required[0]["amount"], "0",
        "zero value_per_unit yields amount 0, not omitted"
    );
    assert_eq!(required[0]["satisfiable"], false);
}

/// When borrower has existing debt and new_loan_amount is 0, required_collateral_value_usd must
/// reflect existing debt (covers "existing + new" per docs), not zero.
#[test]
fn get_collateral_requirements_zero_new_loan_with_existing_debt_returns_required_for_existing_debt()
{
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    // Borrower has 1000 uylds.fcc (6 decimals) existing debt; borrow_index = 1 so underlying = 1000e6.
    let scaled = 1_000_000_000u128; // 1000 * 10^6
    set_scaled_borrow(deps.as_mut().storage, SOME_USER, scaled).expect("set scaled borrow");
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    set_reserve_state_v1(
        deps.as_mut().storage,
        &ReserveStateV1 {
            total_scaled_borrow: scaled,
            ..reserve
        },
    )
    .expect("update reserve");

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: Some(SOME_USER.to_string()),
            new_loan_amount: Uint128::zero(),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetCollateralRequirements response");
    let required_val: &str = res["required_collateral_value_usd"].as_str().unwrap();
    // Debt value = 1000 USD, margin_rate = 0.8 → required = 1000/0.8 = 1250.
    assert!(
        required_val.starts_with("1250"),
        "required_collateral_value_usd should reflect existing debt (1250), got {}",
        required_val
    );
    let additional_val: &str = res["additional_collateral_value_usd"].as_str().unwrap();
    assert!(
        additional_val.starts_with("1250"),
        "additional_collateral_value_usd (no existing collateral) should be 1250, got {}",
        additional_val
    );
}

#[test]
fn get_collateral_requirements_fails_when_oracle_returns_stale_price() {
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        stale_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        stale_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    let err = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: None,
            new_loan_amount: Uint128::new(1000),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .unwrap_err();

    match &err {
        QueryError::Contract(ContractError::StalePriceDataError { .. }) => {}
        _ => panic!("expected StalePriceDataError, got {:?}", err),
    }
}

#[test]
fn get_collateral_requirements_is_not_quotable_when_requested_collateral_is_stale() {
    let (mut deps, env) = setup_instantiated();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        stale_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: None,
            new_loan_amount: Uint128::new(1000),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("stale requested collateral should be unquotable, not a query error");
    let resp: CollateralRequirementsResponseV1 = from_json(bin).unwrap();
    assert!(resp.required[0].amount.is_zero());
    assert!(
        !resp.required[0].satisfiable,
        "stale requested asset is unquotable, not “need none”"
    );
}

#[test]
fn get_collateral_requirements_succeeds_when_held_collateral_price_is_stale() {
    let (mut deps, env) = setup_instantiated();
    let mut amounts = BTreeMap::new();
    amounts.insert("asset.one".to_string(), 10u128);
    set_borrower_collateral(
        deps.as_mut().storage,
        SOME_USER,
        &BorrowerCollateralV1 { amounts },
    )
    .expect("set borrower collateral");

    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        stale_oracle_price(Decimal256::one(), env.block.time),
    );
    set_oracle_prices(&mut deps, prices);

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: Some(SOME_USER.to_string()),
            new_loan_amount: Uint128::zero(),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("stale held collateral must not fail the requirements query");
    let resp: CollateralRequirementsResponseV1 = from_json(bin).unwrap();
    assert!(resp.required[0].amount.is_zero());
    assert!(
        resp.required[0].satisfiable,
        "zero additional needed is a real quote even if the requested feed is stale"
    );
}

/// 18-decimal collateral at $1e-18: required base units exceed u128. Query still
/// returns (amount 0 = no finite representable amount), rather than erroring wholesale.
#[test]
#[allow(deprecated)]
fn get_collateral_requirements_cheap_18_decimal_asset_returns_zero_amount() {
    let (mut deps, env) = setup_instantiated();
    let s = env.block.time.seconds();
    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        AssetPriceResponseV1 {
            price_usd: Decimal256::zero(),
            display_price_usd: Decimal256::from_ratio(1u128, 1_000_000_000_000_000_000u128),
            precision: 18,
            as_of_epoch_second: s,
            expiration_epoch_seconds: s.saturating_add(1),
        },
    );
    set_oracle_prices(&mut deps, prices);

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetCollateralRequirements {
            borrower: None,
            new_loan_amount: Uint128::new(800),
            collateral_assets: vec!["asset.one".to_string()],
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetCollateralRequirements response");
    assert_eq!(res["required_collateral_value_usd"], "1000");
    let required = res["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0]["asset_id"], "asset.one");
    assert_eq!(required[0]["amount"], "0");
    assert_eq!(required[0]["satisfiable"], false);
}
