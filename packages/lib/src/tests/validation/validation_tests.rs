//! Unit tests for pool_v2 utils/validation.rs: migration version, single-coin denom, lender/borrower attributes.

use crate::common::model::error::ContractError;
use crate::common::utils::validation::validate_contract_migration_version;

// ---- validate_contract_migration_version ----

#[test]
fn migration_version_allows_next_greater() {
    assert!(validate_contract_migration_version("1.0.0", "2.0.0").is_ok());
    assert!(validate_contract_migration_version("1.2.0", "1.3.0").is_ok());
    assert!(validate_contract_migration_version("1.2.0", "1.2.1").is_ok());
}

#[test]
fn migration_version_rejects_same() {
    let err = validate_contract_migration_version("1.2.0", "1.2.0").unwrap_err();
    match &err {
        ContractError::UnsupportedUpgrade {
            source_version,
            target_version,
        } => {
            assert_eq!(source_version, "1.2.0");
            assert_eq!(target_version, "1.2.0");
        }
        _ => panic!("expected UnsupportedUpgrade, got {:?}", err),
    }
}

#[test]
fn migration_version_rejects_older() {
    assert!(validate_contract_migration_version("1.2.0", "1.1.0").is_err());
}

#[test]
fn migration_version_rejects_invalid_current() {
    let err = validate_contract_migration_version("not-semver", "1.0.0").unwrap_err();
    match &err {
        ContractError::VersionParseError(s) => assert_eq!(s, "not-semver"),
        _ => panic!("expected VersionParseError, got {:?}", err),
    }
}

#[test]
fn migration_version_rejects_invalid_next() {
    let err = validate_contract_migration_version("1.0.0", "v1.0.0").unwrap_err();
    match &err {
        ContractError::VersionParseError(s) => assert_eq!(s, "v1.0.0"),
        _ => panic!("expected VersionParseError, got {:?}", err),
    }
}
