use crate::constants::{CONTRACT_NAME, CONTRACT_VERSION};
use crate::contract::migrate;
use crate::msg::MigrateMsg;
use crate::storage::get_contract_state_v1;
use crate::tests::instantiate_helpers::setup_instantiated_contract;
use cosmwasm_std::testing::mock_env;
use cosmwasm_std::Addr;
use cw2::{get_contract_version, set_contract_version};
use cw_ownable::get_ownership;
use serde_json::json;

#[test]
fn migration_succeeds_with_legacy_admin_field_when_cw_ownable_missing() {
    let (mut deps, _env) = setup_instantiated_contract();

    // Simulate legacy chain data: `admin` lived in contract state JSON but cw-ownable was never initialized.
    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let mut state_json = serde_json::to_value(&state).unwrap();
    // not the same as OWNER to ensure this test doesn't get a false positive from setup_instantiated_contract
    let legacy_flat_state_admin = "tp1lfglp38atk7gv3z4pg4d3a6m62ma59x6tfwv9p";
    state_json
        .as_object_mut()
        .expect("contract state serializes to a JSON object")
        .insert("admin".to_string(), json!(legacy_flat_state_admin));
    deps.as_mut().storage.set(
        b"cs1",
        &serde_json::to_vec(&state_json).expect("serialize legacy state"),
    );
    deps.as_mut().storage.remove(b"ownership");

    assert!(
        get_ownership(deps.as_ref().storage).is_err(),
        "precondition: no cw-ownable record on chain"
    );

    set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.0.9").unwrap();

    migrate(deps.as_mut(), mock_env(), MigrateMsg {})
        .expect("migrate should initialize owner from legacy flattened-state admin field");

    let ownership = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(
        ownership.owner,
        Some(Addr::unchecked(legacy_flat_state_admin))
    );
    let v = get_contract_version(deps.as_ref().storage).unwrap();
    assert_eq!(v.version, CONTRACT_VERSION);
}
