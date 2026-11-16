//! Fit a stored directed-sale listing to the loan that remains after a debt decrease.
//! Borrow's clear of a new or expired listing stays in `borrow.rs`.

use crate::model::directed_sale::{
    clamp_to_balances, scale_to_cap, DirectedSaleOfferV1, ScaleToCap,
};
use crate::model::error::ContractError;
use crate::model::ContractStateV1;
use crate::storage::{clear_directed_sale_offer, get_borrower_collateral, set_directed_sale_offer};
use crate::utils::{get_asset_prices_for_borrower, scaled_to_underlying_borrow_ceil};
use cosmwasm_std::{Decimal256, QuerierWrapper, Storage, Timestamp, Uint128};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use std::collections::BTreeMap;

/// Amounts to fit, and the scaled debt those amounts have to stay under.
pub struct ListingAfterDebt {
    pub new_scaled: u128,
    pub borrow_index: Decimal256,
    pub basis: BTreeMap<String, Uint128>,
    pub expires_at: Timestamp,
}

/// Clamp `listing.basis` to the collateral just saved, then shrink it to the payoff cap.
///
/// `new_scaled == 0` deletes the row and does not read prices. A price error, a
/// listed asset with no fresh non-zero price, or a math overflow keeps the clamped
/// amounts. The stored nonce and `expires_at` are left as they are.
pub fn sync_directed_sale_after_debt_decrease(
    storage: &mut dyn Storage,
    querier: &QuerierWrapper,
    block_time: Timestamp,
    contract: &ContractStateV1,
    borrower: &str,
    listing: ListingAfterDebt,
) -> Result<(), ContractError> {
    if listing.new_scaled == 0 {
        return clear_directed_sale_offer(storage, borrower);
    }
    let balances = get_borrower_collateral(storage, borrower)?;
    let clamped = clamp_to_balances(&listing.basis, &balances.amounts);
    if clamped.is_empty() {
        return clear_directed_sale_offer(storage, borrower);
    }
    let prices = match get_asset_prices_for_borrower(querier, &block_time, contract, &balances) {
        Ok(prices) => prices,
        Err(_) => return write_amounts(storage, borrower, clamped, listing.expires_at),
    };
    let Some(cap_usd) = payoff_cap_usd(&prices, contract, listing.new_scaled, listing.borrow_index)
    else {
        return write_amounts(storage, borrower, clamped, listing.expires_at);
    };
    let mut amount_value_usd = BTreeMap::new();
    for (asset_id, amount) in &clamped {
        let Some(price) = prices.get(asset_id) else {
            return write_amounts(storage, borrower, clamped, listing.expires_at);
        };
        if price.is_zero_price() {
            return write_amounts(storage, borrower, clamped, listing.expires_at);
        }
        let Ok(value) = price.value_usd(amount.u128()) else {
            return write_amounts(storage, borrower, clamped, listing.expires_at);
        };
        if value.is_zero() {
            return write_amounts(storage, borrower, clamped, listing.expires_at);
        }
        amount_value_usd.insert(asset_id.clone(), value);
    }
    match scale_to_cap(&clamped, &amount_value_usd, cap_usd) {
        ScaleToCap::Unchanged => write_amounts(storage, borrower, clamped, listing.expires_at),
        ScaleToCap::Scaled(amounts) => {
            write_amounts(storage, borrower, amounts, listing.expires_at)
        }
    }
}

/// `ceil(scaled × borrow_index)` in lending-denom USD, times `liquidation_bonus_rate`.
/// `None` when the lending price or the multiplication cannot be used.
fn payoff_cap_usd(
    prices: &PriceMapResponse,
    contract: &ContractStateV1,
    new_scaled: u128,
    borrow_index: Decimal256,
) -> Option<Decimal256> {
    let price: &AssetPriceResponseV1 = prices.get(&contract.lending_denom.name)?;
    if price.is_zero_price() {
        return None;
    }
    let payoff = scaled_to_underlying_borrow_ceil(new_scaled, borrow_index).ok()?;
    let payoff_usd = price.value_usd(payoff).ok()?;
    payoff_usd.checked_mul(contract.liquidation_bonus_rate).ok()
}

fn write_amounts(
    storage: &mut dyn Storage,
    borrower: &str,
    amounts: BTreeMap<String, Uint128>,
    expires_at: Timestamp,
) -> Result<(), ContractError> {
    if amounts.is_empty() {
        return clear_directed_sale_offer(storage, borrower);
    }
    set_directed_sale_offer(
        storage,
        borrower,
        &DirectedSaleOfferV1 {
            amounts,
            expires_at,
        },
    )
}
