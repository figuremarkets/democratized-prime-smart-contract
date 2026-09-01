//! GetBorrowerPosition: full borrower view (debt, collateral amounts, collateral value USD, LTV, health).
use crate::model::error::QueryError;
use crate::model::health::BorrowerHealthResponseV1;
use crate::model::query::{AssetRequirementV1, BorrowerPositionResponseV1};
use crate::storage::{get_borrower_collateral, get_contract_state_v1, get_scaled_borrow};
use crate::utils::health::{calculate_total_collateral_value_usd, get_borrower_health};
use crate::utils::{
    compute_effective_reserve, get_asset_prices_for_borrower, scaled_to_underlying_borrow,
};
use cosmwasm_std::{to_json_binary, Binary, Deps, Env, Uint128};

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

    let collateral: Vec<AssetRequirementV1> = borrower_collateral
        .amounts
        .iter()
        .map(|(id, amt)| AssetRequirementV1 {
            asset_id: id.clone(),
            amount: Uint128::from(*amt),
        })
        .collect();

    let (collateral_value_usd, loan_to_value, health, health_unknown_reason) =
        if borrower_collateral.amounts.is_empty() {
            (
                "0".to_string(),
                "0".to_string(),
                if underlying_debt == 0 {
                    BorrowerHealthResponseV1::Healthy
                } else {
                    BorrowerHealthResponseV1::NoCollateral
                },
                None,
            )
        } else {
            let prices = get_asset_prices_for_borrower(
                &deps.querier,
                &env.block.time,
                &contract,
                &borrower_collateral,
            )
            .map_err(QueryError::Contract)?;

            let collateral_value = calculate_total_collateral_value_usd(
                &borrower_collateral,
                &prices,
                &contract.supported_collateral_assets,
            )
            .map_err(QueryError::Contract)?;
            let collateral_value_usd = collateral_value.to_string();

            let debt_u128 = Uint128::from(underlying_debt);
            let health_result = get_borrower_health(
                &contract,
                &contract.supported_collateral_assets,
                &prices,
                &borrower_collateral,
                debt_u128,
            );
            match health_result {
                Ok((h, ltv)) => (
                    collateral_value_usd,
                    ltv.to_string(),
                    BorrowerHealthResponseV1::from(h),
                    None,
                ),
                Err(e) => (
                    collateral_value_usd,
                    "0".to_string(),
                    BorrowerHealthResponseV1::Unknown,
                    Some(e.to_string()),
                ),
            }
        };

    to_json_binary(&BorrowerPositionResponseV1 {
        address: address.to_string(),
        scaled_borrow: scaled.to_string(),
        underlying_debt: underlying_debt.to_string(),
        underlying_debt_display: contract.lending_denom.base_to_display(underlying_debt)?,
        lending_denom: contract.lending_denom,
        collateral,
        collateral_value_usd,
        loan_to_value,
        health,
        health_unknown_reason,
    })
    .map_err(QueryError::Std)
}
