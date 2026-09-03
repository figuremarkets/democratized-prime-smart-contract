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

/// Validate a configured required-attribute pattern (exact name or leading `*.suffix` wildcard).
pub fn validate_required_attr_pattern(name: &str) -> Result<(), ContractError> {
    ensure!(
        !name.is_empty(),
        illegal_argument("required attribute name cannot be empty")
    );
    if name.contains('*') {
        let suffix = name.strip_prefix("*.").ok_or_else(|| {
            illegal_argument(format!(
                "invalid wildcard attribute pattern \"{name}\": only leading *.suffix is supported"
            ))
        })?;
        ensure!(
            !suffix.is_empty(),
            illegal_argument("wildcard pattern *.suffix must have a non-empty suffix")
        );
        ensure!(
            !suffix.contains('*'),
            illegal_argument("wildcard pattern may contain only one leading *")
        );
    }
    Ok(())
}

/// Validate every entry in a required-attribute list.
pub fn validate_required_attr_patterns(attrs: &[String]) -> Result<(), ContractError> {
    for attr in attrs {
        validate_required_attr_pattern(attr)?;
    }
    Ok(())
}

/// True when `attr_name` satisfies a leading-wildcard pattern `*.suffix` (e.g. `figure.kyb.pb` for `*.kyb.pb`).
fn attribute_matches_wildcard_suffix(attr_name: &str, suffix: &str) -> bool {
    attr_name.len() > suffix.len()
        && attr_name.ends_with(suffix)
        && attr_name.as_bytes().get(attr_name.len() - suffix.len() - 1) == Some(&b'.')
}

/// Check whether `account` has the required attribute (exact name or `*.suffix` wildcard).
fn account_has_required_attribute<Q: cosmwasm_std::CustomQuery>(
    q: &AttributeQuerier<'_, Q>,
    account: &str,
    required: &str,
) -> Result<bool, ContractError> {
    if let Some(suffix) = required.strip_prefix("*.") {
        let res = q.scan(account.to_string(), suffix.to_string(), None)?;
        Ok(res
            .attributes
            .iter()
            .any(|a| attribute_matches_wildcard_suffix(&a.name, suffix)))
    } else {
        let res = q.attribute(account.to_string(), required.to_string(), None)?;
        Ok(!res.attributes.is_empty())
    }
}

fn validate_account_attrs(
    querier: &QuerierWrapper,
    account: &str,
    required_attrs: &[String],
    role: &str,
) -> Result<(), ContractError> {
    if required_attrs.is_empty() {
        return Ok(());
    }
    let q = AttributeQuerier::new(querier);
    for attr in required_attrs {
        ensure!(
            account_has_required_attribute(&q, account, attr)?,
            not_authorized(format!(
                "Missing required {role} attribute; must have all of: [{}]",
                required_attrs.join(", ")
            ))
        );
    }
    Ok(())
}

/// Ensure sender has all of the required lender attributes. Empty list = no check (anyone can lend).
pub fn validate_lender_attrs(
    querier: &QuerierWrapper,
    sender: &str,
    required_attrs: &[String],
) -> Result<(), ContractError> {
    validate_account_attrs(querier, sender, required_attrs, "lender")
}

/// Ensure sender has all of the required borrower attributes. Empty list = no check (anyone can borrow).
pub fn validate_borrower_attrs(
    querier: &QuerierWrapper,
    sender: &str,
    required_attrs: &[String],
) -> Result<(), ContractError> {
    validate_account_attrs(querier, sender, required_attrs, "borrower")
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
