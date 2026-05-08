//! LTV and borrower health (margin rate = healthy bound; liquidation rate = liquidatable threshold).

use crate::model::collateral::BorrowerCollateralV1;
use crate::model::collateral::CollateralAssetV1;
use crate::model::contract_state::ContractStateV1;
use crate::model::error::{illegal_argument, not_found, ContractError};
use crate::model::haircut_percentage;
use crate::model::health::BorrowerHealthV1;
use crate::utils::{format_as_percent_string, uint128_to_decimal256};
use cosmwasm_std::{ensure, Decimal256, Uint128};
use democratized_prime_lib::price_oracle::model::PriceMapResponse;
use result_extensions::ResultExtensions;

/// Computes health state and LTV for a borrower given collateral, prices, and debt (underlying amount).
pub fn get_borrower_health(
    contract_state: &ContractStateV1,
    supported_assets: &[CollateralAssetV1],
    asset_prices: &PriceMapResponse,
    borrower_collateral: &BorrowerCollateralV1,
    underlying_debt: Uint128,
) -> Result<(BorrowerHealthV1, Decimal256), ContractError> {
    let loan_to_value = calculate_ltv(
        contract_state,
        supported_assets,
        asset_prices,
        borrower_collateral,
        underlying_debt,
    )?;
    let health = get_health_from_ltv(contract_state, loan_to_value)?;
    (health, loan_to_value).to_ok()
}

pub fn get_health_from_ltv(
    contract_state: &ContractStateV1,
    loan_to_value: Decimal256,
) -> Result<BorrowerHealthV1, ContractError> {
    if loan_to_value >= contract_state.liquidation_rate {
        return BorrowerHealthV1::Liquidatable.to_ok();
    }
    // Unhealthy only when LTV is strictly above margin_rate; LTV == margin_rate is Healthy.
    if loan_to_value > contract_state.margin_rate {
        return BorrowerHealthV1::Unhealthy.to_ok();
    }
    BorrowerHealthV1::Healthy.to_ok()
}

/// LTV = debt_value_usd / collateral_value_usd. Errors if no collateral but debt > 0.
pub fn calculate_ltv(
    contract_state: &ContractStateV1,
    supported_assets: &[CollateralAssetV1],
    asset_prices: &PriceMapResponse,
    borrower_collateral: &BorrowerCollateralV1,
    underlying_debt: Uint128,
) -> Result<Decimal256, ContractError> {
    let borrow_balance_usd = calculate_borrow_value_usd(
        underlying_debt,
        &contract_state.lending_denom.name,
        asset_prices,
    )?;
    let total_collateral_value_usd =
        calculate_total_collateral_value_usd(borrower_collateral, asset_prices, supported_assets)?;

    if borrow_balance_usd.is_zero() && total_collateral_value_usd.is_zero() {
        return Decimal256::zero().to_ok();
    }
    if total_collateral_value_usd.is_zero() {
        return illegal_argument(format!(
            "No collateral for loans [debt value {}]",
            borrow_balance_usd
        ))
        .to_err();
    }
    // Guard above ensures no divide-by-zero (e.g. when all collateral prices are 0).
    borrow_balance_usd
        .checked_div(total_collateral_value_usd)
        .map_err(ContractError::from)
}

pub fn calculate_total_collateral_value_usd(
    collateral: &BorrowerCollateralV1,
    prices: &PriceMapResponse,
    supported_assets: &[CollateralAssetV1],
) -> Result<Decimal256, ContractError> {
    let mut total = Decimal256::zero();
    for (asset_id, amount) in collateral.amounts.iter() {
        let asset_price_usd = prices
            .get(asset_id)
            .ok_or_else(|| not_found(format!("Price of asset: {}", asset_id)))
            .map(|r| r.price_usd)?;
        let haircut = haircut_percentage(supported_assets, asset_id);
        let value = asset_price_usd
            .checked_mul(uint128_to_decimal256(*amount))
            .map_err(ContractError::from)?
            .checked_mul(haircut)
            .map_err(ContractError::from)?;
        total += value;
    }
    total.to_ok()
}

pub fn calculate_borrow_value_usd(
    underlying_debt: Uint128,
    lending_denom: &str,
    prices: &PriceMapResponse,
) -> Result<Decimal256, ContractError> {
    if underlying_debt.is_zero() {
        return Decimal256::zero().to_ok();
    }
    let price_usd = prices
        .get(lending_denom)
        .ok_or_else(|| not_found(format!("Price of asset: {}", lending_denom)))
        .map(|r| r.price_usd)?;

    ensure!(
        !price_usd.is_zero(),
        illegal_argument("Lending denom price is zero")
    );

    price_usd
        .checked_mul(uint128_to_decimal256(underlying_debt.u128()))
        .map_err(ContractError::from)
}

/// Ensures the borrower is Healthy (LTV <= margin_rate). Used before allowing borrow.
pub fn validate_borrower_is_healthy(
    health: BorrowerHealthV1,
    loan_to_value: Decimal256,
    contract_state: &ContractStateV1,
) -> Result<(), ContractError> {
    if health != BorrowerHealthV1::Healthy {
        let threshold = format_as_percent_string(match health {
            BorrowerHealthV1::Healthy => contract_state.margin_rate,
            BorrowerHealthV1::Unhealthy => contract_state.margin_rate,
            BorrowerHealthV1::Liquidatable => contract_state.liquidation_rate,
        })?;
        let ltv_str = format_as_percent_string(loan_to_value)?;
        return illegal_argument(format!(
            "Resulting Loan-to-value [{ltv_str}] is above {health:?} threshold [{threshold}]"
        ))
        .to_err();
    }
    ().to_ok()
}
