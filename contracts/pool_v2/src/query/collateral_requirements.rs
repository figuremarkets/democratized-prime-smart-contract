//! "Required collateral" query for UI: "how much collateral do I need to borrow X?"
//!
//! Returns required collateral value (USD, after haircuts) and per-asset minimum amounts.
//! When `borrower` is set: existing debt is included in the required total, and existing
//! collateral is subtracted so per-asset amounts are the *additional* collateral needed.
//! Held denoms omitted from that credit are named in `unpriceable_collateral`.
//! Per-asset `satisfiable` is false when a positive requirement could not be quoted
//! (deliberately coarse: down feed vs zero haircut vs unrepresentable units).

use crate::model::error::{ContractError, QueryError};
use crate::model::query::AssetRequirementV1;
use crate::model::{haircut_percentage, CollateralRequirementsResponseV1};
use crate::storage::{get_borrower_collateral, get_contract_state_v1, get_scaled_borrow};
use crate::utils::health::calculate_total_collateral_value_usd;
use crate::utils::{
    calculate_borrow_value_usd, compute_effective_reserve, drop_unpriceable_collateral,
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
                .map(|id| AssetRequirementV1::quoted(id.clone(), Uint128::zero()))
                .collect(),
            unpriceable_collateral: Vec::new(),
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
    let mut prices = get_price_from_oracle(
        &deps.querier,
        &contract.price_oracle_address,
        &asset_ids,
        true,
    )
    .map_err(QueryError::Contract)?;
    drop_unpriceable_collateral(&mut prices, &contract.lending_denom.name, &env.block.time)
        .map_err(QueryError::Contract)?;

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
        // Need none of every asset — a missing feed is not a failed quote.
        if value_to_cover.is_zero() {
            required.push(AssetRequirementV1::quoted(
                asset_id.clone(),
                Uint128::zero(),
            ));
            continue;
        }
        let haircut = haircut_percentage(&contract.supported_collateral_assets, asset_id);
        let req = match prices.get(asset_id) {
            None => AssetRequirementV1::unquotable(asset_id.clone()),
            Some(price) if price.is_zero_price() || haircut.is_zero() => {
                AssetRequirementV1::unquotable(asset_id.clone())
            }
            Some(price) => {
                // units * (display / 10^precision) * haircut >= value_to_cover
                let pre_haircut = value_to_cover
                    .checked_div(haircut)
                    .map_err(|e| QueryError::Contract(ContractError::from(e)))?;
                match price.amount_from_usd(pre_haircut) {
                    Ok(amt) => AssetRequirementV1::quoted(asset_id.clone(), Uint128::from(amt)),
                    // Cheap high-precision asset: required base units do not fit u128.
                    Err(ContractError::AmountNotRepresentable) => {
                        AssetRequirementV1::unquotable(asset_id.clone())
                    }
                    Err(e) => return Err(QueryError::Contract(e)),
                }
            }
        };
        required.push(req);
    }

    let unpriceable_collateral = borrower_collateral_opt
        .as_ref()
        .map(|bc| {
            bc.amounts
                .keys()
                .filter(|id| !prices.contains_key(*id))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    to_json_binary(&CollateralRequirementsResponseV1 {
        required_collateral_value_usd: required_collateral_value_usd.to_string(),
        additional_collateral_value_usd: value_to_cover.to_string(),
        required,
        unpriceable_collateral,
    })
    .map_err(QueryError::Std)
}
