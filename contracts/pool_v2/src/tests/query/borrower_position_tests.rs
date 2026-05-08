//! Tests for GetBorrowerPosition query (debt, collateral, value, LTV, health).

use crate::contract::query;
use crate::model::error::{ContractError, QueryError};
use crate::model::{BorrowerCollateralV1, ReserveStateV1};
use crate::msg::QueryMsg;
use crate::storage::{
    get_reserve_state_v1, set_borrower_collateral, set_reserve_state_v1, set_scaled_borrow,
};
use crate::tests::fixtures::stale_oracle_price;
use crate::tests::query::common::{setup_instantiated, ORACLE, SOME_USER};
use cosmwasm_std::{
    from_json, to_json_binary, ContractResult, Decimal256, QuerierResult, SystemError,
    SystemResult, WasmQuery,
};
use democratized_prime_lib::price_oracle::model::AssetPriceResponseV1;
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use std::collections::HashMap;
use std::str::FromStr;

fn set_oracle_prices(
    deps: &mut cosmwasm_std::OwnedDeps<
        cosmwasm_std::MemoryStorage,
        cosmwasm_std::testing::MockApi,
        provwasm_mocks::MockProvenanceQuerier,
    >,
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
    deps.querier.mock_querier.update_wasm(handler);
}

#[test]
fn get_borrower_position_zero_when_no_borrow() {
    let (deps, env) = setup_instantiated();
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetBorrowerPosition {
            address: SOME_USER.to_string(),
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetBorrowerPosition response");
    assert_eq!(res["address"], SOME_USER);
    assert_eq!(res["scaled_borrow"].as_str(), Some("0"));
    assert_eq!(res["underlying_debt"].as_str(), Some("0"));
    assert_eq!(res["underlying_debt_display"].as_str(), Some("0.000000"));
    assert_eq!(res["lending_denom"]["n"].as_str(), Some("uylds.fcc"));
    assert_eq!(res["lending_denom"]["p"].as_u64(), Some(6));
    assert!(res["collateral"].as_array().unwrap().is_empty());
    assert_eq!(res["collateral_value_usd"].as_str(), Some("0"));
    assert_eq!(res["loan_to_value"].as_str(), Some("0"));
    assert_eq!(res["health"].as_str(), Some("healthy"));
}

#[test]
fn get_borrower_position_returns_scaled_and_underlying() {
    let (mut deps, env) = setup_instantiated();
    let borrower = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";
    let scaled = 1_000_000u128;
    set_scaled_borrow(deps.as_mut().storage, borrower, scaled).expect("set scaled borrow");
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    set_reserve_state_v1(
        deps.as_mut().storage,
        &ReserveStateV1 {
            borrow_index: Decimal256::from_str("1.05").unwrap(),
            ..reserve
        },
    )
    .expect("update reserve");
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetBorrowerPosition {
            address: borrower.to_string(),
        },
    )
    .expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetBorrowerPosition response");
    assert_eq!(res["address"], borrower);
    assert_eq!(res["scaled_borrow"].as_str(), Some("1000000"));
    assert_eq!(res["underlying_debt"].as_str(), Some("1050000"));
    assert_eq!(res["underlying_debt_display"].as_str(), Some("1.050000"));
    assert!(res["collateral"].as_array().unwrap().is_empty());
    assert_eq!(res["health"].as_str(), Some("no_collateral"));
}

#[test]
fn get_borrower_position_fails_invalid_address() {
    let (deps, env) = setup_instantiated();
    let err = query(
        deps.as_ref(),
        env,
        QueryMsg::GetBorrowerPosition {
            address: "not-a-valid-address".to_string(),
        },
    )
    .unwrap_err();
    match &err {
        QueryError::Std(_) => {}
        _ => panic!(
            "expected Std error from addr_validate for invalid address, got {:?}",
            err
        ),
    }
}

#[test]
fn get_borrower_position_fails_when_oracle_returns_stale_price() {
    let (mut deps, env) = setup_instantiated();
    let mut amounts = std::collections::BTreeMap::new();
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
        QueryMsg::GetBorrowerPosition {
            address: SOME_USER.to_string(),
        },
    )
    .unwrap_err();

    match &err {
        QueryError::Contract(ContractError::StalePriceDataError { .. }) => {}
        _ => panic!(
            "expected StalePriceDataError wrapped in QueryError, got {:?}",
            err
        ),
    }
}
