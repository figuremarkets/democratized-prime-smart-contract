//! Contract owner only: set the list of required lender attributes (lend / transfer recipient must have all).

use crate::constants::MAX_LENDER_BORROWER_REQUIRED_ATTRS;
use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_LENDER_REQUIRED_ATTRS_JSON};
use crate::model::error::illegal_argument;
use crate::model::error::{invalid_funds, ContractError};
use crate::storage::{get_contract_state_v1, set_contract_state_v1};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::assert_owner;
use result_extensions::ResultExtensions;

pub const ACTION: &str = "set_lender_required_attrs";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may set lender required attributes";

/// Set lender required attributes. Contract owner only; no funds. Empty list = no attribute check.
pub fn set_lender_required_attrs(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    lender_required_attrs: Vec<String>,
) -> Result<Response, ContractError> {
    let mut contract = get_contract_state_v1(deps.storage)?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));
    ensure!(
        lender_required_attrs.len() <= MAX_LENDER_BORROWER_REQUIRED_ATTRS,
        illegal_argument(format!(
            "No more than [{}] lender required attributes allowed",
            MAX_LENDER_BORROWER_REQUIRED_ATTRS
        ))
    );
    contract.lender_required_attrs = lender_required_attrs.clone();
    set_contract_state_v1(deps.storage, &contract)?;
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(
            ATTRIBUTE_LENDER_REQUIRED_ATTRS_JSON,
            serde_json::to_string(&lender_required_attrs).unwrap_or_default(),
        )
        .to_ok()
}
