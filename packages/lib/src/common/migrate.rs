use crate::common::constants::ATTRIBUTE_ACTION_NAME;
use crate::common::model::error::{illegal_argument, illegal_state, not_found, ContractError};
use crate::common::utils::validate_contract_migration_version;
use cosmwasm_std::{ensure, Api, Response, Storage};
use cw2::{get_contract_version, set_contract_version};
use cw_ownable::{get_ownership, initialize_owner};
use cw_storage_plus::Item;
use result_extensions::ResultExtensions;
use serde::Deserialize;

const ACTION: &str = "migrate";

/// JSON shape used when migrating from optional legacy `admin` in flattened contract state
/// to [`cw_ownable`] ownership storage.
#[derive(Debug, Deserialize)]
pub struct LegacyAdminFlattenState<S> {
    #[serde(default)]
    pub admin: Option<cosmwasm_std::Addr>,
    #[serde(flatten)]
    pub state: S,
}

/// When [`Some`], strip legacy `admin` from JSON at `key` and initialize cw-ownable if needed.
/// Pair with [`migrate_contract`] using the state type parameter `S` for that key (for example
/// pool contract state).
pub struct LegacyMigration<'a, S> {
    pub item: &'a Item<S>,
    pub api: &'a dyn Api,
}

/// Migrate to a new version. The code version ([`contract_version`]) must be strictly
/// greater than the version stored on-chain.
///
/// When `legacy` is [`Some`], loads JSON at `key` as [`LegacyAdminFlatten<S>`], writes back
/// only `state` (no legacy `admin` field), and seeds [`cw_ownable`] from `admin` when present
/// and ownership storage is missing.
///
/// For contracts without legacy admin in state, use [`LegacyMigration`] absent:
/// `migrate_contract::<()>(store, name, version, None)`.
pub fn migrate_contract<S>(
    store: &mut dyn Storage,
    contract_name: &str,
    contract_version: &str,
    legacy: Option<LegacyMigration<'_, S>>,
) -> Result<Response, ContractError>
where
    S: serde::de::DeserializeOwned + serde::Serialize,
{
    let v = get_contract_version(store).map_err(ContractError::Std)?;
    ensure!(
        v.contract == contract_name,
        illegal_argument(format!(
            "Expected contract name {} got {}",
            contract_name, v.contract
        ))
    );
    validate_contract_migration_version(&v.version, contract_version)?;
    set_contract_version(store, contract_name, contract_version)?;

    if let Some(legacy) = legacy {
        migrate_legacy_admin_from_flattened_state::<S>(store, legacy)?;
    }

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .to_ok()
}

fn migrate_legacy_admin_from_flattened_state<S>(
    store: &mut dyn Storage,
    legacy: LegacyMigration<'_, S>,
) -> Result<(), ContractError>
where
    S: serde::de::DeserializeOwned + serde::Serialize,
{
    if !legacy.item.exists(store) {
        return Ok(());
    }
    // load contract state with legacy admin
    let bytes = store
        .get(legacy.item.as_slice())
        .ok_or_else(|| not_found("contract state"))?;
    let migrated: LegacyAdminFlattenState<S> =
        cosmwasm_std::from_json(&bytes).map_err(ContractError::Std)?;
    // re-store contract state without legacy admin
    legacy
        .item
        .save(store, &migrated.state)
        .map_err(ContractError::Std)?;

    if let Some(admin) = migrated.admin {
        if get_ownership(store).is_err() {
            initialize_owner(store, legacy.api, Some(admin.as_str()))
                .map_err(ContractError::Std)?;
        }
    } else {
        ensure!(
            get_ownership(store)
                .map_err(ContractError::Std)?
                .owner
                .is_some(),
            illegal_state(
                "migrate: contract state has no legacy admin and ownership is missing; cannot recover owner",
            )
        );
    }

    Ok(())
}
