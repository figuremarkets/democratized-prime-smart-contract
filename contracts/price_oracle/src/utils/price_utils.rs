use crate::constants::TEN;
use cosmwasm_std::Decimal256;
use democratized_prime_lib::common::ContractError;
use result_extensions::ResultExtensions;
use std::str::FromStr;

/// Scale the price by 10^precision
///
/// # Example
/// display_price = 100000
/// base_precision = 3
/// display_price / 10^base_precision = 100000 / 10^3 = 100
///
pub fn scale_price(
    display_price: Decimal256,
    base_precision: u32,
) -> Result<Decimal256, ContractError> {
    let divisor: Decimal256 = Decimal256::from_str(TEN)?.checked_pow(base_precision)?;
    display_price.checked_div(divisor)?.to_ok()
}
