use crate::constants::{ATTRIBUTE_CUSTODIAN, CONTRACT_NAME, CONTRACT_VERSION};
use crate::contract::{migrate, ASSERT_CUSTODIAN_ERR};
use crate::model::error::illegal_argument;
use crate::model::ContractStateV1;
use crate::msg::MigrateMsg;
use crate::storage::contract_state_key;
use crate::storage::{get_contract_state_v1, set_contract_state_v1};
use crate::tests::instantiate_helpers::{setup_instantiated_contract, CUSTODIAN, OWNER};
use cosmwasm_std::testing::{mock_env, MockApi};
use cosmwasm_std::{Addr, Attribute, DepsMut};
use cw2::{get_contract_version, set_contract_version};
use cw_ownable::{get_ownership, initialize_owner};
use democratized_prime_lib::common::migrate::ACTION as MIGRATE_ACTION;
use democratized_prime_lib::common::{ContractError, ATTRIBUTE_ACTION_NAME};
use provwasm_mocks::mock_provenance_dependencies;
use serde_json::json;

const TEST_REPO_TOKEN_CONTRACT_ADDRESS: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const TEST_PRICE_ORACLE_CONTRACT_ADDRESS: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
const PREVIOUS_CONTRACT_VERSION: &str = "0.1.0";

fn simulate_legacy_contract(deps: DepsMut<'_>, api: MockApi) -> Result<(), ContractError> {
    // omit "c_a" (custodian account attribute):
    let json_contract_legacy_state: String = format!(
        r#"
        {{
            "atca": "{TEST_REPO_TOKEN_CONTRACT_ADDRESS}",
            "bdla": "deferred_to_deficit",
            "bra":
            [],
            "c_n": "pool-v2-demo",
            "commit_market_id": null,
            "d": "Test pool v2",
            "lbr": "1.02",
            "ld":
            {{
                "n": "uylds.fcc",
                "p": 6
            }},
            "lr": "0.9",
            "lra":
            [],
            "max_borrower_collateral_types": 5,
            "min_borrow": "1",
            "min_lend": "1",
            "mr": "0.8",
            "op": "active",
            "poa": "{TEST_PRICE_ORACLE_CONTRACT_ADDRESS}",
            "rp":
            {{
                "kink": "0.9",
                "maxr": "0.2",
                "minr": "0.0325",
                "rf": "0.005",
                "spy": 31536000,
                "tr": "0.09"
            }},
            "sca":
            [
                {{
                    "h": "0.8",
                    "id": "asset.one"
                }}
            ]
        }}
        "#
    );
    // Simulate an older version of the contract in storage that
    // doesn't have its "c_a" custodian account property set:
    let legacy_state: ContractStateV1 = serde_json::from_str(&json_contract_legacy_state)?;
    set_contract_state_v1(deps.storage, &legacy_state)?;
    // Simulate a version increase from 0.1.0 to *CURRENT_VERSION* (must be greater than 0.1.0 + defined in Cargo.toml):
    set_contract_version(deps.storage, CONTRACT_NAME, "0.1.0")?;
    initialize_owner(deps.storage, &api, Some(OWNER))?;

    Ok(())
}

#[test]
fn migration_succeeds_with_legacy_admin_field_when_cw_ownable_missing() {
    let (mut deps, _env) = setup_instantiated_contract();

    // Simulate legacy chain data: `admin` lived in contract state JSON but cw-ownable was never initialized.
    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let contract_state_key = contract_state_key();
    let mut state_json = serde_json::to_value(&state).unwrap();

    // not the same as OWNER to ensure this test doesn't get a false positive from setup_instantiated_contract
    let legacy_flat_state_admin = "tp1lfglp38atk7gv3z4pg4d3a6m62ma59x6tfwv9p";
    state_json
        .as_object_mut()
        .expect("contract state serializes to a JSON object")
        .insert("admin".to_string(), json!(legacy_flat_state_admin));
    deps.as_mut().storage.set(
        &contract_state_key.as_bytes(),
        &serde_json::to_vec(&state_json).expect("serialize legacy state"),
    );
    deps.as_mut().storage.remove(b"ownership");

    assert!(
        get_ownership(deps.as_ref().storage).is_err(),
        "precondition: no cw-ownable record on chain"
    );

    set_contract_version(
        deps.as_mut().storage,
        CONTRACT_NAME,
        PREVIOUS_CONTRACT_VERSION,
    )
    .unwrap();

    migrate(deps.as_mut(), mock_env(), MigrateMsg { custodian: None })
        .expect("migrate should initialize owner from legacy flattened-state admin field");

    let ownership = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(
        ownership.owner,
        Some(Addr::unchecked(legacy_flat_state_admin))
    );
    // Custodian is updated for the contract:
    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(state.custodian, Some(Addr::unchecked(CUSTODIAN)));
    let v = get_contract_version(deps.as_ref().storage).unwrap();
    assert_eq!(v.version, CONTRACT_VERSION);
}

#[test]
fn migration_fails_custodian_if_contract_custodian_is_not_currently_set() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let api = deps.api;

    simulate_legacy_contract(deps.as_mut(), api).unwrap();

    // Attempt the migration - no custodian
    let res = migrate(deps.as_mut(), mock_env(), MigrateMsg { custodian: None });

    assert_eq!(res, Err(illegal_argument(ASSERT_CUSTODIAN_ERR)));

    // Verify the custodian was not updated:
    let state: ContractStateV1 = get_contract_state_v1(deps.as_mut().storage).unwrap();
    assert_eq!(state.custodian, None);

    let v = get_contract_version(deps.as_ref().storage).unwrap();
    // Version not updated:
    assert_eq!(v.version, PREVIOUS_CONTRACT_VERSION);
}

#[test]
fn migration_fails_whitespace_only_custodian() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let api = deps.api;

    simulate_legacy_contract(deps.as_mut(), api).unwrap();

    let err = migrate(
        deps.as_mut(),
        mock_env(),
        MigrateMsg {
            custodian: Some("   ".to_owned()),
        },
    )
    .unwrap_err();

    assert!(matches!(err, ContractError::Std(_)));

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(state.custodian, None);

    let v = get_contract_version(deps.as_ref().storage).unwrap();
    // Version not updated:
    assert_eq!(v.version, PREVIOUS_CONTRACT_VERSION);
}

#[test]
fn migration_succeeds_when_custodian_already_set() {
    let (mut deps, _env) = setup_instantiated_contract();

    set_contract_version(
        deps.as_mut().storage,
        CONTRACT_NAME,
        PREVIOUS_CONTRACT_VERSION,
    )
    .unwrap();

    // The migration should succeed when custodian already set:
    migrate(deps.as_mut(), mock_env(), MigrateMsg { custodian: None }).unwrap();

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(state.custodian, Some(Addr::unchecked(CUSTODIAN)));
    let v = get_contract_version(deps.as_ref().storage).unwrap();
    assert_eq!(v.version, CONTRACT_VERSION);
}

const NEW_CUSTODIAN: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";

#[test]
fn migration_overwrites_existing_custodian() {
    let (mut deps, _env) = setup_instantiated_contract();

    set_contract_version(
        deps.as_mut().storage,
        CONTRACT_NAME,
        PREVIOUS_CONTRACT_VERSION,
    )
    .unwrap();

    let res = migrate(
        deps.as_mut(),
        mock_env(),
        MigrateMsg {
            custodian: Some(NEW_CUSTODIAN.to_owned()),
        },
    )
    .unwrap();

    assert_eq!(
        res.attributes,
        vec![
            Attribute::new(ATTRIBUTE_ACTION_NAME, MIGRATE_ACTION),
            Attribute::new(ATTRIBUTE_CUSTODIAN, NEW_CUSTODIAN)
        ]
    );

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(state.custodian, Some(Addr::unchecked(NEW_CUSTODIAN)));
}

#[test]
fn migration_fails_invalid_custodian_address() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let api = deps.api;

    simulate_legacy_contract(deps.as_mut(), api).unwrap();

    let err = migrate(
        deps.as_mut(),
        mock_env(),
        MigrateMsg {
            custodian: Some("not_a_valid_address".to_owned()),
        },
    )
    .unwrap_err();

    assert!(matches!(err, ContractError::Std(_)));

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(state.custodian, None);

    let v = get_contract_version(deps.as_ref().storage).unwrap();
    assert_eq!(v.version, PREVIOUS_CONTRACT_VERSION);
}

#[test]
fn migration_legacy_admin_preserves_existing_custodian_without_msg() {
    let (mut deps, _env) = setup_instantiated_contract();

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let mut state_json = serde_json::to_value(&state).unwrap();
    let contract_state_key = contract_state_key();
    let legacy_flat_state_admin = "tp1lfglp38atk7gv3z4pg4d3a6m62ma59x6tfwv9p";
    state_json
        .as_object_mut()
        .expect("contract state serializes to a JSON object")
        .insert("admin".to_string(), json!(legacy_flat_state_admin));
    deps.as_mut().storage.set(
        contract_state_key.as_bytes(),
        &serde_json::to_vec(&state_json).unwrap(),
    );
    deps.as_mut().storage.remove(b"ownership");

    set_contract_version(
        deps.as_mut().storage,
        CONTRACT_NAME,
        PREVIOUS_CONTRACT_VERSION,
    )
    .unwrap();

    migrate(deps.as_mut(), mock_env(), MigrateMsg { custodian: None })
        .expect("migrate with existing custodian should succeed");

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(state.custodian, Some(Addr::unchecked(CUSTODIAN)));

    let ownership = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(
        ownership.owner,
        Some(Addr::unchecked(legacy_flat_state_admin))
    );
}

#[test]
fn migrate_msg_json_deserializes_custodian() {
    let msg: MigrateMsg = serde_json::from_str(&format!(r#"{{"custodian":"{CUSTODIAN}"}}"#))
        .expect("deserialize MigrateMsg");
    assert_eq!(msg.custodian, Some(CUSTODIAN.to_owned()));

    let empty: MigrateMsg = serde_json::from_str("{}").expect("deserialize empty MigrateMsg");
    assert_eq!(empty.custodian, None);
}

#[test]
fn migration_proceeds_when_custodian_is_specified() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let api = deps.api;

    simulate_legacy_contract(deps.as_mut(), api).unwrap();

    // Attempt the migration - with a new custodian
    let res = migrate(
        deps.as_mut(),
        mock_env(),
        MigrateMsg {
            custodian: Some(CUSTODIAN.to_owned()),
        },
    )
    .unwrap();

    assert_eq!(
        res.attributes,
        vec![
            Attribute::new(ATTRIBUTE_ACTION_NAME, MIGRATE_ACTION),
            Attribute::new(ATTRIBUTE_CUSTODIAN, CUSTODIAN)
        ]
    );

    // Verify the custodian was updated:
    let state: ContractStateV1 = get_contract_state_v1(deps.as_mut().storage).unwrap();
    assert_eq!(state.custodian, Some(Addr::unchecked(CUSTODIAN)));
}
