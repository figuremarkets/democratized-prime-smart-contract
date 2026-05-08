use crate::common::constants::ATTRIBUTE_ACTION_NAME;
use crate::common::model::error::{illegal_argument, invalid_funds, not_authorized, ContractError};
use cosmwasm_std::{ensure, Addr, DepsMut, Env, MessageInfo, Response, Storage};
use cw_ownable::{update_ownership as cw_update_ownership, Action, OwnershipError};
use result_extensions::ResultExtensions;

/// Wrapper around [`cw_ownable::assert_owner`]: on [`OwnershipError::NotOwner`] or
/// [`OwnershipError::NoOwner`], returns [`not_authorized(not_owner_message)`] instead of
/// [`ContractError::Ownership`], matching legacy admin-check behavior.
pub fn assert_owner(
    storage: &dyn Storage,
    sender: &Addr,
    not_owner_message: impl AsRef<str>,
) -> Result<(), ContractError> {
    let msg = not_owner_message.as_ref();
    cw_ownable::assert_owner(storage, sender).map_err(|e| match e {
        OwnershipError::NotOwner | OwnershipError::NoOwner => not_authorized(msg),
        other => ContractError::Ownership(other),
    })
}

/// Value of the primary [`ATTRIBUTE_ACTION_NAME`] response attribute for ownership changes.
pub const UPDATE_OWNERSHIP_ACTION: &str = "update_ownership";

/// No funds; applies cw-ownable [`Action`]; renouncing ownership is rejected; returns ownership
/// attributes on the response.
pub fn update_ownership(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    action: Action,
) -> Result<Response, ContractError> {
    ensure!(
        info.funds.is_empty(),
        invalid_funds("No funds accepted for ownership updates")
    );
    ensure!(
        !matches!(action, Action::RenounceOwnership),
        illegal_argument("Renouncing contract ownership is not supported")
    );
    let ownership = cw_update_ownership(deps, &env.block, &info.sender, action)?;
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, UPDATE_OWNERSHIP_ACTION)
        .add_attributes(
            ownership
                .into_attributes()
                .into_iter()
                .map(|a| (a.key, a.value)),
        )
        .to_ok()
}
