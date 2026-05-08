use crate::model::error::{illegal_argument, illegal_state, not_authorized, ContractError};
use crate::model::{BorrowerCollateralV1, Denom};
use cosmwasm_std::{ensure, Coin, MessageInfo, QuerierWrapper, Uint128};
use provwasm_std::types::provenance::attribute::v1::AttributeQuerier;
use std::collections::HashSet;
use std::convert::TryInto;

/// Ensure exactly one coin sent and it matches lending denom; return its amount.
pub fn validate_single_coin_denom(
    info: &MessageInfo,
    lending_denom: &Denom,
    min_amount: Uint128,
) -> Result<Uint128, ContractError> {
    let coins = &info.funds;
    ensure!(
        coins.len() == 1,
        illegal_argument("Exactly one coin must be sent")
    );
    let coin = &coins[0];
    ensure!(
        coin.denom == lending_denom.name,
        illegal_argument(format!("Expected denom {}", lending_denom.name))
    );
    ensure!(
        coin.amount >= min_amount,
        illegal_argument(format!(
            "Amount {} below minimum {}",
            coin.amount, min_amount
        ))
    );
    Ok(coin.amount)
}

/// Ensure sender has all of the required lender attributes. Empty list = no check (anyone can lend).
pub fn validate_lender_attrs(
    querier: &QuerierWrapper,
    sender: &str,
    required_attrs: &[String],
) -> Result<(), ContractError> {
    if required_attrs.is_empty() {
        return Ok(());
    }
    let q = AttributeQuerier::new(querier);
    for attr in required_attrs {
        let res = q.attribute(sender.to_string(), attr.clone(), None)?;
        ensure!(
            !res.attributes.is_empty(),
            not_authorized(format!(
                "Missing required lender attribute; must have all of: [{}]",
                required_attrs.join(", ")
            ))
        );
    }
    Ok(())
}

/// Ensure sender has all of the required borrower attributes. Empty list = no check (anyone can borrow).
pub fn validate_borrower_attrs(
    querier: &QuerierWrapper,
    sender: &str,
    required_attrs: &[String],
) -> Result<(), ContractError> {
    if required_attrs.is_empty() {
        return Ok(());
    }
    let q = AttributeQuerier::new(querier);
    for attr in required_attrs {
        let res = q.attribute(sender.to_string(), attr.clone(), None)?;
        ensure!(
            !res.attributes.is_empty(),
            not_authorized(format!(
                "Missing required borrower attribute; must have all of: [{}]",
                required_attrs.join(", ")
            ))
        );
    }
    Ok(())
}

/// Validate that the number of distinct collateral asset types (new + existing) does not exceed the limit.
pub fn validate_borrower_collateral_type_limit(
    new_collateral: &[Coin],
    existing_collateral: &BorrowerCollateralV1,
    max_types: u32,
) -> Result<(), ContractError> {
    let mut distinct: HashSet<String> = new_collateral.iter().map(|c| c.denom.clone()).collect();
    distinct.extend(existing_collateral.amounts.keys().cloned());
    let n: u32 = distinct
        .len()
        .try_into()
        .map_err(|_| illegal_state("Too many collateral types"))?;
    ensure!(
        n <= max_types,
        illegal_argument(format!(
            "Too many collateral types provided (total [{}] limit: [{}])",
            n, max_types
        ))
    );
    Ok(())
}
