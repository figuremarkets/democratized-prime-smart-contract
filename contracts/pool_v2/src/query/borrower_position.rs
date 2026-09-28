//! GetBorrowerPosition: full borrower view (debt, collateral amounts, unpriceable denoms,
//! collateral value USD, borrow-side LTV/health, liquidation LTV/health).
use crate::model::{
    health::BorrowerHealthResponseV1, AssetRequirementV1, BorrowerCollateralV1,
    BorrowerPositionResponseV1, ContractStateV1, QueryError,
};
use crate::storage::{get_borrower_collateral, get_contract_state_v1, get_scaled_borrow};
use crate::utils::{
    calculate_total_collateral_value_usd, compute_effective_reserve, drop_unpriceable_collateral,
    drop_unpriceable_for_liquidation, get_borrower_health, get_price_from_oracle,
    scaled_to_underlying_borrow,
};
use cosmwasm_std::{to_json_binary, Binary, Deps, Env, Uint128};
use democratized_prime_lib::price_oracle::model::PriceMapResponse;

/// Returns debt, collateral amounts, collateral value (USD), LTV, and health for a borrower.
pub fn query_borrower_position(deps: Deps, env: Env, address: &str) -> Result<Binary, QueryError> {
    deps.api.addr_validate(address).map_err(QueryError::Std)?;
    let contract = get_contract_state_v1(deps.storage).map_err(QueryError::Contract)?;
    let reserve = compute_effective_reserve(deps.storage, env.block.time, &contract.rate_params)
        .map_err(QueryError::Contract)?;

    let scaled = get_scaled_borrow(deps.storage, address).map_err(QueryError::Contract)?;
    let underlying_debt =
        scaled_to_underlying_borrow(scaled, reserve.borrow_index).map_err(QueryError::Contract)?;

    let borrower_collateral =
        get_borrower_collateral(deps.storage, address).map_err(QueryError::Contract)?;

    let mut collateral: Vec<AssetRequirementV1> = Vec::new();
    let mut unpriceable_collateral = Vec::new();
    let mut liquidation_unpriceable_collateral = Vec::new();
    let (
        collateral_value_usd,
        loan_to_value,
        health,
        health_unknown_reason,
        liquidation_ltv,
        liquidation_health,
    ) = if borrower_collateral.amounts.is_empty() {
        let health = if underlying_debt == 0 {
            BorrowerHealthResponseV1::Healthy
        } else {
            BorrowerHealthResponseV1::NoCollateral
        };
        (
            "0".to_string(),
            "0".to_string(),
            health.clone(),
            None,
            "0".to_string(),
            health,
        )
    } else {
        let lending_denom = &contract.lending_denom.name;
        let mut asset_ids: Vec<String> = vec![lending_denom.clone()];
        asset_ids.extend(borrower_collateral.amounts.keys().cloned());

        let raw_prices = get_price_from_oracle(
            &deps.querier,
            &contract.price_oracle_address,
            &asset_ids,
            true,
        )
        .map_err(QueryError::Contract)?;

        // Borrow-side filter first (strict `is_stale`). That success implies lending is
        // fresh, so the looser liquidation last-known check cannot be the binding failure.
        let mut prices = raw_prices.clone();
        drop_unpriceable_collateral(&mut prices, lending_denom, &env.block.time)
            .map_err(QueryError::Contract)?;
        let liquidation_prices = drop_unpriceable_for_liquidation(
            raw_prices,
            &env.block.time,
            &contract,
            &borrower_collateral,
        )
        .map_err(QueryError::Contract)?;

        unpriceable_collateral = borrower_collateral
            .amounts
            .keys()
            .filter(|id| !prices.contains_key(*id))
            .cloned()
            .collect();
        liquidation_unpriceable_collateral = borrower_collateral
            .amounts
            .keys()
            .filter(|id| liquidation_prices.unpriceable.contains(*id))
            .cloned()
            .collect();
        collateral = borrower_collateral
            .amounts
            .iter()
            .map(|(id, amt)| {
                AssetRequirementV1::holding(
                    id.clone(),
                    Uint128::from(*amt),
                    prices.contains_key(id),
                )
            })
            .collect();

        let collateral_value = calculate_total_collateral_value_usd(
            &borrower_collateral,
            &prices,
            &contract.supported_collateral_assets,
        )
        .map_err(QueryError::Contract)?;
        let collateral_value_usd = collateral_value.to_string();

        let debt_u128 = Uint128::from(underlying_debt);
        let (loan_to_value, health, health_unknown_reason) =
            health_from_prices(&contract, &prices, &borrower_collateral, debt_u128);
        let (liquidation_ltv, liquidation_health, _) = health_from_prices(
            &contract,
            &liquidation_prices.prices,
            &borrower_collateral,
            debt_u128,
        );

        (
            collateral_value_usd,
            loan_to_value,
            health,
            health_unknown_reason,
            liquidation_ltv,
            liquidation_health,
        )
    };

    to_json_binary(&BorrowerPositionResponseV1 {
        address: address.to_string(),
        scaled_borrow: scaled.to_string(),
        underlying_debt: underlying_debt.to_string(),
        underlying_debt_display: contract.lending_denom.base_to_display(underlying_debt)?,
        lending_denom: contract.lending_denom,
        collateral,
        unpriceable_collateral,
        collateral_value_usd,
        loan_to_value,
        health,
        health_unknown_reason,
        liquidation_ltv,
        liquidation_health,
        liquidation_unpriceable_collateral,
    })
    .map_err(QueryError::Std)
}

fn health_from_prices(
    contract: &ContractStateV1,
    prices: &PriceMapResponse,
    borrower_collateral: &BorrowerCollateralV1,
    debt_u128: Uint128,
) -> (String, BorrowerHealthResponseV1, Option<String>) {
    match get_borrower_health(
        contract,
        &contract.supported_collateral_assets,
        prices,
        borrower_collateral,
        debt_u128,
    ) {
        Ok((h, ltv)) => (ltv.to_string(), BorrowerHealthResponseV1::from(h), None),
        Err(e) => (
            "0".to_string(),
            BorrowerHealthResponseV1::Unknown,
            Some(e.to_string()),
        ),
    }
}
