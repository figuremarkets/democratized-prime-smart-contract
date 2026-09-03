//! Contract owner only: set the list of required borrower attributes (Borrow / AddCollateral / RemoveCollateral require all).

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_BORROWER_REQUIRED_ATTRS_JSON,
    MAX_LENDER_BORROWER_REQUIRED_ATTRS,
};
use crate::model::error::{illegal_argument, invalid_funds, ContractError};
use crate::model::ContractStateV1;
use crate::storage::{get_contract_state_v1, set_contract_state_v1};
use crate::utils::{assert_custodian, validate_required_attr_patterns};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response};
use result_extensions::ResultExtensions;

pub const ACTION: &str = "set_borrower_required_attrs";
pub const ASSERT_CUSTODIAN_ERR: &str =
    "Only the contract custodian may set borrower required attributes";

/// Set borrower required attributes. Contract owner only; no funds. Empty list = no attribute check.
/// Sender must have all attributes in the list to Borrow, AddCollateral, or RemoveCollateral.
pub fn set_borrower_required_attrs(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    borrower_required_attrs: Vec<String>,
) -> Result<Response, ContractError> {
    let mut contract: ContractStateV1 = get_contract_state_v1(deps.storage)?;
    assert_custodian(&contract, &info.sender, ASSERT_CUSTODIAN_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));
    ensure!(
        borrower_required_attrs.len() <= MAX_LENDER_BORROWER_REQUIRED_ATTRS,
        illegal_argument(format!(
            "No more than [{}] borrower required attributes allowed",
            MAX_LENDER_BORROWER_REQUIRED_ATTRS
        ))
    );
    validate_required_attr_patterns(&borrower_required_attrs)?;
    contract.borrower_required_attrs = borrower_required_attrs.clone();
    set_contract_state_v1(deps.storage, &contract)?;
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(
            ATTRIBUTE_BORROWER_REQUIRED_ATTRS_JSON,
            serde_json::to_string(&borrower_required_attrs).unwrap_or_default(),
        )
        .to_ok()
}
