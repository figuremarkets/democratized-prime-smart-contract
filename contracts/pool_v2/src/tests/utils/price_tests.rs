//! Unit tests for pool_v2 utils/price.rs.
//!
//! Oracle query functions require a mocked querier returning `PriceMapResponse`.
//! Successful paths for `get_price_from_oracle` / `get_asset_prices_for_borrower` use a mocked oracle;
//! `expiration_epoch_seconds` is derived from [`cosmwasm_std::testing::mock_env`] block time (`setup_instantiated`).

use crate::model::collateral::BorrowerCollateralV1;
use crate::model::error::ContractError;
use crate::storage::{get_contract_state_v1, set_borrower_collateral};
use crate::tests::fixtures::{fresh_oracle_price, stale_oracle_price};
use crate::tests::query::common::{setup_instantiated, ORACLE, SOME_USER};
use crate::utils::{get_asset_prices_for_borrower, get_price_from_oracle};
use cosmwasm_std::{
    from_json, to_json_binary, Addr, ContractResult, Decimal256, QuerierResult, SystemError,
    SystemResult, WasmQuery,
};
use democratized_prime_lib::price_oracle::model::AssetPriceResponseV1;
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use provwasm_mocks::mock_provenance_dependencies;
use std::collections::{BTreeMap, HashMap};

/// Without mocking the oracle, `get_price_from_oracle` fails (querier has no wasm handler for the oracle).
#[test]
fn get_price_from_oracle_fails_when_oracle_not_mocked() {
    let deps = mock_provenance_dependencies();
    let oracle = Addr::unchecked("oracle");
    let assets = vec!["lend".to_string(), "btc".to_string()];

    let err = get_price_from_oracle(&deps.as_ref().querier, &oracle, &assets).unwrap_err();

    match &err {
        ContractError::Std(_) => {}
        _ => panic!(
            "expected Std error from querier when oracle not mocked, got {:?}",
            err
        ),
    }
}

/// Mock queries to the price oracle, returning the price fixture data given in [`prices`].
fn mock_price_oracle(
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
fn get_asset_prices_for_borrower_succeeds_when_prices_fresh() {
    let (mut deps, env) = setup_instantiated();
    let mut amounts = BTreeMap::new();
    amounts.insert("asset.one".to_string(), 1u128);
    let collateral = BorrowerCollateralV1 { amounts };
    set_borrower_collateral(deps.as_mut().storage, SOME_USER, &collateral).unwrap();

    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    mock_price_oracle(&mut deps, prices);

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();

    let result = get_asset_prices_for_borrower(
        &deps.as_ref().querier,
        &env.block.time,
        &contract,
        &collateral,
    );
    assert!(result.is_ok());
}

#[test]
fn get_asset_prices_for_borrower_fails_when_any_price_is_stale() {
    let (mut deps, env) = setup_instantiated();
    let mut amounts = BTreeMap::new();
    amounts.insert("asset.one".to_string(), 1u128);
    let collateral = BorrowerCollateralV1 { amounts };
    set_borrower_collateral(deps.as_mut().storage, SOME_USER, &collateral).unwrap();

    let mut prices = HashMap::new();
    prices.insert(
        "uylds.fcc".to_string(),
        fresh_oracle_price(Decimal256::one(), env.block.time),
    );
    prices.insert(
        "asset.one".to_string(),
        stale_oracle_price(Decimal256::one(), env.block.time),
    );
    mock_price_oracle(&mut deps, prices);

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();

    let err = get_asset_prices_for_borrower(
        &deps.as_ref().querier,
        &env.block.time,
        &contract,
        &collateral,
    )
    .unwrap_err();

    match &err {
        ContractError::StalePriceDataError { asset_id, .. } => {
            assert_eq!(asset_id, "asset.one");
        }
        _ => panic!("expected StalePriceDataError, got {:?}", err),
    }
}
