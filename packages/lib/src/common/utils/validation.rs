use crate::common::ContractError;
use cosmwasm_std::ensure;
use result_extensions::ResultExtensions;
use semver::Version;

/// Ensures the migration target version is strictly greater than the current on-chain version.
pub fn validate_contract_migration_version(
    current_version: &str,
    next_version: &str,
) -> Result<(), ContractError> {
    let current = Version::parse(current_version)
        .map_err(|_| ContractError::VersionParseError(current_version.to_owned()))?;
    let next = Version::parse(next_version)
        .map_err(|_| ContractError::VersionParseError(next_version.to_owned()))?;
    ensure!(
        next > current,
        ContractError::UnsupportedUpgrade {
            source_version: current.to_string(),
            target_version: next.to_string(),
        }
    );
    ().to_ok()
}
