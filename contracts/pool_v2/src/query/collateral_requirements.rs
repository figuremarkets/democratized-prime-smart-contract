//! "Required collateral" query for UI: "how much collateral do I need to borrow X?"
//!
//! Returns required collateral value (USD, after haircuts) and per-asset minimum amounts.
//! When `borrower` is set: existing debt is included in the required total, and existing
//! collateral is subtracted so per-asset amounts are the *additional* collateral needed.

use crate::model::error::{not_found, ContractError, QueryError};
use crate::model::query::AssetRequirementV1;
use crate::model::{haircut_percentage, CollateralRequirementsResponseV1};
use crate::storage::{get_borrower_collateral, get_contract_state_v1, get_scaled_borrow};
use crate::utils::health::calculate_total_collateral_value_usd;
use crate::utils::{
    calculate_borrow_value_usd, compute_effective_reserve, decimal256_ceil_to_u128,
    get_price_from_oracle, scaled_to_underlying_borrow,
};
use cosmwasm_std::{to_json_binary, Binary, Decimal256, Deps, Env, Uint128};

/// Returns required collateral value (USD) and per-asset minimum amounts for the given borrower and/or new loan. See module doc.
pub fn query_collateral_requirements(
    deps: Deps,
    env: Env,
    borrower: Option<&str>,
    new_loan_amount: Uint128,
    collateral_assets: &[String],
) -> Result<Binary, QueryError> {
    let contract = get_contract_state_v1(deps.storage).map_err(QueryError::Contract)?;

    // No borrower and no new loan → no debt to cover; return zeros without calling the oracle.
    // When borrower is Some we always run the full path so existing debt is included (required_collateral_value_usd = existing + new).
    if new_loan_amount.is_zero() && borrower.is_none() {
        return to_json_binary(&CollateralRequirementsResponseV1 {
            required_collateral_value_usd: "0".to_string(),
            additional_collateral_value_usd: "0".to_string(),
            required: collateral_assets
                .iter()
                .map(|id| AssetRequirementV1 {
                    asset_id: id.clone(),
                    amount: Uint128::zero(),
                })
                .collect(),
        })
        .map_err(QueryError::Std);
    }

    let mut asset_ids = vec![contract.lending_denom.name.clone()];
    for id in collateral_assets {
        if !asset_ids.contains(id) {
            asset_ids.push(id.clone());
        }
    }
    let borrower_collateral_opt = borrower
        .map(|addr| get_borrower_collateral(deps.storage, addr))
        .transpose()
        .map_err(QueryError::Contract)?;
    if let Some(ref bc) = borrower_collateral_opt {
        for id in bc.amounts.keys() {
            if !asset_ids.contains(id) {
                asset_ids.push(id.clone());
            }
        }
    }
    let prices = get_price_from_oracle(&deps.querier, &contract.price_oracle_address, &asset_ids)
        .map_err(QueryError::Contract)?;

    // Check the price data for staleness:
    for (asset_id, price) in prices.iter() {
        if price.is_stale(env.block.time) {
            return Err(ContractError::StalePriceDataError {
                asset_id: asset_id.clone(),
                expired_at: price.expired_at(),
            }
            .into());
        }
    }

    let new_loan_value_usd =
        calculate_borrow_value_usd(new_loan_amount, &contract.lending_denom.name, &prices)
            .map_err(QueryError::Contract)?;

    let current_loan_value_usd = if let Some(addr) = borrower {
        let reserve =
            compute_effective_reserve(deps.storage, env.block.time, &contract.rate_params)
                .map_err(QueryError::Contract)?;
        let scaled = get_scaled_borrow(deps.storage, addr).map_err(QueryError::Contract)?;
        let existing_underlying = scaled_to_underlying_borrow(scaled, reserve.borrow_index)
            .map_err(QueryError::Contract)?;
        calculate_borrow_value_usd(
            Uint128::from(existing_underlying),
            &contract.lending_denom.name,
            &prices,
        )
        .map_err(QueryError::Contract)?
    } else {
        Decimal256::zero()
    };

    let total_debt_value_usd = current_loan_value_usd
        .checked_add(new_loan_value_usd)
        .map_err(|e| QueryError::Contract(ContractError::from(e)))?;
    // Minimum collateral so that LTV <= margin_rate (Healthy). get_borrower_health treats LTV > margin_rate as Unhealthy.
    let required_collateral_value_usd = total_debt_value_usd
        .checked_div(contract.margin_rate)
        .map_err(|e| QueryError::Contract(ContractError::from(e)))?;

    // When borrower is set, subtract their existing collateral value so per-asset "required" is additional needed.
    let value_to_cover = if let Some(ref bc) = borrower_collateral_opt {
        let existing_collateral_value = calculate_total_collateral_value_usd(
            bc,
            &prices,
            &contract.supported_collateral_assets,
        )
        .map_err(QueryError::Contract)?;
        required_collateral_value_usd
            .checked_sub(existing_collateral_value)
            .unwrap_or(Decimal256::zero())
    } else {
        required_collateral_value_usd
    };

    let mut required: Vec<AssetRequirementV1> = Vec::with_capacity(collateral_assets.len());
    for asset_id in collateral_assets {
        let haircut = haircut_percentage(&contract.supported_collateral_assets, asset_id);
        let price_usd = prices
            .get(asset_id)
            .ok_or_else(|| QueryError::Contract(not_found(format!("Price of asset: {}", asset_id))))
            .map(|r| r.price_usd)?;
        let value_per_unit = price_usd
            .checked_mul(haircut)
            .map_err(|e| QueryError::Contract(ContractError::from(e)))?;
        let amount = if value_per_unit.is_zero() {
            // Asset has zero value per unit (e.g. price or haircut is 0); no finite amount satisfies. Preserve 1:1 with collateral_assets.
            Uint128::zero()
        } else {
            let amount_d = value_to_cover
                .checked_div(value_per_unit)
                .map_err(|e| QueryError::Contract(ContractError::from(e)))?;
            Uint128::from(decimal256_ceil_to_u128(amount_d).unwrap_or(0))
        };
        required.push(AssetRequirementV1 {
            asset_id: asset_id.clone(),
            amount,
        });
    }

    to_json_binary(&CollateralRequirementsResponseV1 {
        required_collateral_value_usd: required_collateral_value_usd.to_string(),
        additional_collateral_value_usd: value_to_cover.to_string(),
        required,
    })
    .map_err(QueryError::Std)
}
