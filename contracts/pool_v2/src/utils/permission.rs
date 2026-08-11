use crate::model::error::ContractError;
use crate::model::ContractStateV1;
use crate::storage::get_contract_state_v1;
use cosmwasm_std::{ensure, Addr, Storage};
use democratized_prime_lib::common::is_owner;

/// Asserts the sender is the designated custodian for the contract.
pub fn assert_custodian(
    contract_state: &ContractStateV1,
    sender: &Addr,
    not_custodian_message: impl AsRef<str>,
) -> Result<(), ContractError> {
    let custodian: Addr = match contract_state.custodian {
        Some(ref custodian) => custodian.to_owned(),
        None => {
            return Err(ContractError::NotAuthorizedError {
                message: "contract custodian not set".to_owned(),
            })
        }
    };
    ensure!(
        sender == custodian,
        ContractError::NotAuthorizedError {
            message: not_custodian_message.as_ref().to_owned()
        }
    );
    Ok(())
}

/// Asserts the sender is either the contract owner or the designated custodian
/// for the contract.
pub fn assert_owner_or_custodian(
    store: &dyn Storage,
    sender: &Addr,
    not_permitted_message: impl AsRef<str>,
) -> Result<(), ContractError> {
    // Check: is contract owner?
    if is_owner(store, sender)? {
        return Ok(());
    }
    // Check: is contract custodian?
    let contract_state: ContractStateV1 = get_contract_state_v1(store)?;
    let custodian: Addr = match contract_state.custodian {
        Some(ref custodian) => custodian.to_owned(),
        None => {
            return Err(ContractError::NotAuthorizedError {
                message: "contract custodian not set".to_owned(),
            })
        }
    };
    ensure!(
        sender == custodian,
        ContractError::NotAuthorizedError {
            message: not_permitted_message.as_ref().to_owned()
        }
    );
    Ok(())
}
