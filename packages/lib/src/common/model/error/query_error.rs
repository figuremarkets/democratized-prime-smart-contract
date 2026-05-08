use crate::common::model::error::contract_error::ContractError;
use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum QueryError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Contract(#[from] ContractError),

    // Not found resource
    #[error("not found: {message}")]
    NotFoundError { message: String },

    // Not allowed input parameters
    #[error("illegal argument: {message}")]
    IllegalArgumentError { message: String },

    // Something bad state thing happened
    #[error("execution error: {message}")]
    IllegalStateError { message: String },
}
