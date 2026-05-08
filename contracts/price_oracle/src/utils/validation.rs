use crate::model::error::{illegal_argument, ContractError};
use cosmwasm_std::ensure;
use result_extensions::ResultExtensions;
use std::collections::HashSet;

/// Validates the uniqueness of the given names, reporting the duplicate as
/// an error.
pub fn validate_name_uniqueness(names: &Vec<String>) -> Result<(), ContractError> {
    let mut seen: HashSet<String> = HashSet::new();

    for name in names {
        ensure!(
            !seen.contains(name),
            illegal_argument(format!("Duplicate name: {name}"))
        );
        seen.insert(name.clone());
    }

    ().to_ok()
}
