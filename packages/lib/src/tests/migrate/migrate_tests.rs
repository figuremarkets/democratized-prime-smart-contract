//! Tests for contract migration: contract name must match, new version must be
//! strictly greater than stored version; success updates cw2 version and returns attribute.

use crate::common::constants::ATTRIBUTE_ACTION_NAME;
use crate::common::migrate::migrate_contract;
use crate::common::model::error::ContractError;
use cosmwasm_std::Response;
use cw2::{get_contract_version, set_contract_version};
use provwasm_mocks::mock_provenance_dependencies;
pub const CONTRACT_NAME: &str = "test-contract";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn migration_fails_when_same_version() {
    let mut deps = mock_provenance_dependencies();
    let on_chain_version = CONTRACT_VERSION.to_string();
    set_contract_version(deps.as_mut().storage, CONTRACT_NAME, &on_chain_version).unwrap();

    let result =
        migrate_contract::<()>(deps.as_mut().storage, CONTRACT_NAME, CONTRACT_VERSION, None);

    assert!(matches!(
        result,
        Err(ContractError::UnsupportedUpgrade {
            source_version: _,
            target_version: _
        })
    ));
    let err = result.unwrap_err();
    match &err {
        ContractError::UnsupportedUpgrade {
            source_version,
            target_version,
        } => {
            assert_eq!(source_version, &on_chain_version);
            assert_eq!(target_version, CONTRACT_VERSION);
        }
        _ => panic!("expected UnsupportedUpgrade"),
    }
}

#[test]
fn migration_fails_when_stored_version_newer_than_code() {
    let mut deps = mock_provenance_dependencies();
    set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "2.0.0").unwrap();

    let result =
        migrate_contract::<()>(deps.as_mut().storage, CONTRACT_NAME, CONTRACT_VERSION, None);

    assert!(matches!(
        result,
        Err(ContractError::UnsupportedUpgrade {
            source_version: _,
            target_version: _
        })
    ));
}

#[test]
fn migration_fails_when_contract_name_mismatch() {
    let mut deps = mock_provenance_dependencies();
    set_contract_version(deps.as_mut().storage, "other-contract-name", "0.0.9").unwrap();

    let result =
        migrate_contract::<()>(deps.as_mut().storage, CONTRACT_NAME, CONTRACT_VERSION, None);

    match &result {
        Err(ContractError::IllegalArgumentError { message }) => {
            assert!(message.contains("Expected contract name"));
            assert!(message.contains(CONTRACT_NAME));
            assert!(message.contains("other-contract-name"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", result),
    }
}

#[test]
fn migration_succeeds_when_stored_version_older() {
    // let (mut deps, _env) = setup_instantiated_contract();
    let mut deps = mock_provenance_dependencies();

    let on_chain_version = "0.0.9";
    set_contract_version(deps.as_mut().storage, CONTRACT_NAME, on_chain_version).unwrap();

    let result =
        migrate_contract::<()>(deps.as_mut().storage, CONTRACT_NAME, CONTRACT_VERSION, None)
            .expect("migrate should succeed");

    assert_eq!(
        result,
        Response::new().add_attribute(ATTRIBUTE_ACTION_NAME, "migrate")
    );
    let v = get_contract_version(deps.as_ref().storage).unwrap();
    assert_eq!(v.contract, CONTRACT_NAME);
    assert_eq!(v.version, CONTRACT_VERSION);
}

#[test]
fn migration_succeeds_with_older_patch_version() {
    let mut deps = mock_provenance_dependencies();
    set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.0.1").unwrap();

    let result =
        migrate_contract::<()>(deps.as_mut().storage, CONTRACT_NAME, CONTRACT_VERSION, None)
            .expect("migrate should succeed");

    assert_eq!(result.attributes.len(), 1);
    assert_eq!(result.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(result.attributes[0].value, "migrate");
    let v = get_contract_version(deps.as_ref().storage).unwrap();
    assert_eq!(v.version, CONTRACT_VERSION);
}
