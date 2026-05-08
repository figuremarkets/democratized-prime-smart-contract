pub mod contract_error;
pub mod query_error;

// re-export
pub use contract_error::{
    illegal_argument, illegal_state, invalid_funds, not_authorized, not_found, ContractError,
};
pub use query_error::QueryError;
