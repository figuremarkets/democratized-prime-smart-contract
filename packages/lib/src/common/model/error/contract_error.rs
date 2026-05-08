use cosmwasm_std::{
    CheckedFromRatioError, ConversionOverflowError, Decimal256RangeExceeded, DecimalRangeExceeded,
    DivideByZeroError, OverflowError, StdError, Timestamp,
};
use cw_ownable::OwnershipError;
use serde_json;
use std::num::TryFromIntError;
use std::string::FromUtf8Error;

// Error types
#[derive(thiserror::Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Ownership(#[from] OwnershipError),

    #[error("{0}")]
    FromUtf8Error(#[from] FromUtf8Error),

    #[error("{0}")]
    TryFromIntError(#[from] TryFromIntError),

    #[error("{0}")]
    DecimalRangeExceeded(#[from] DecimalRangeExceeded),

    #[error("{0}")]
    Decimal256RangeExceeded(#[from] Decimal256RangeExceeded),

    #[error("Overflow or invalid conversion: {0}")]
    Overflow(String),

    #[error("{0}")]
    OverFlowError(#[from] OverflowError),

    #[error("parse error")]
    UuidError(#[from] uuid::Error),

    #[error("serialization error")]
    SerdeError { message: String },

    #[error("{0}")]
    ConversionOverFlowError(#[from] ConversionOverflowError),

    #[error("{0}")]
    CheckedFromRatioError(#[from] CheckedFromRatioError),

    #[error("{0}")]
    DivideByZeroError(#[from] DivideByZeroError),

    // Not authorized to take action error
    #[error("Not authorized: {message}")]
    NotAuthorizedError { message: String },

    // Funds do not match required
    #[error("Invalid funds: {message}")]
    InvalidFundsError { message: String },

    // Not found resource
    #[error("Not found: {message}")]
    NotFoundError { message: String },

    // Not allowed input parameters
    #[error("Illegal argument: {message}")]
    IllegalArgumentError { message: String },

    // Something bad state thing happened
    #[error("Execution error: {message}")]
    IllegalStateError { message: String },

    #[error("Version parse error: [{0}]")]
    VersionParseError(String),

    #[error("Invalid address: {message}")]
    InvalidAddress { message: String },

    #[error("Pool not configured: set pool_address first")]
    PoolNotConfigured,

    // The price data for an asset is too ol
    #[error("Stale price data for {asset_id}; expired at {expired_at}")]
    StalePriceDataError {
        asset_id: String,
        expired_at: Timestamp,
    },

    // Bad contract version when upgrading:
    #[error("Unsupported upgrade: {source_version:?} => {target_version:?}")]
    UnsupportedUpgrade {
        source_version: String,
        target_version: String,
    },
}

pub fn not_authorized<S: AsRef<str>>(message: S) -> ContractError {
    ContractError::NotAuthorizedError {
        message: message.as_ref().to_owned(),
    }
}

pub fn invalid_funds<S: AsRef<str>>(message: S) -> ContractError {
    ContractError::InvalidFundsError {
        message: message.as_ref().to_owned(),
    }
}

pub fn not_found<S: AsRef<str>>(message: S) -> ContractError {
    ContractError::NotFoundError {
        message: message.as_ref().to_owned(),
    }
}

pub fn illegal_argument<S: AsRef<str>>(message: S) -> ContractError {
    ContractError::IllegalArgumentError {
        message: message.as_ref().to_owned(),
    }
}

pub fn illegal_state<S: AsRef<str>>(message: S) -> ContractError {
    ContractError::IllegalStateError {
        message: message.as_ref().to_owned(),
    }
}

impl From<serde_json::Error> for ContractError {
    fn from(err: serde_json::Error) -> Self {
        ContractError::SerdeError {
            message: err.to_string(),
        }
    }
}
