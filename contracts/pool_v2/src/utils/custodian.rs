use crate::model::error::ContractError;
use crate::model::ContractStateV1;
use cosmwasm_std::{ensure, Addr};

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
