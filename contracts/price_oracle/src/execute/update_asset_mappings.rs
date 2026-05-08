use crate::constants::ATTRIBUTE_ACTION_NAME;
use crate::model::{error::ContractError, SaveAssetMappingRequestV1};
use crate::storage::{remove_asset_mapping_v1, save_asset_mapping_v1};
use crate::utils::validate_name_uniqueness;
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::assert_owner;
use result_extensions::ResultExtensions;

pub const ATTRIBUTE_ACTION_VALUE: &str = "update_asset_mappings";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may update asset mappings";

/// Defines and removes asset mappings in the price oracle contract.
///
/// # Arguments
///
/// * `to_update` - Updates to asset mappings via [`SaveAssetMappingRequestV1`].
/// * `to_remove` - Remove the named asset mappings.
pub fn try_update_asset_mappings(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    to_update: Vec<SaveAssetMappingRequestV1>,
    to_remove: Vec<String>,
) -> Result<Response, ContractError> {
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;

    for alt_asset_id_to_remove in to_remove {
        remove_asset_mapping_v1(deps.storage, alt_asset_id_to_remove);
    }

    // Validate unique asset IDs:
    let alt_asset_ids = to_update
        .iter()
        .map(|asset| asset.alt_asset_id.clone())
        .collect::<Vec<_>>();

    validate_name_uniqueness(&alt_asset_ids)?;

    for alt_asset_id_to_update in to_update {
        alt_asset_id_to_update.validate()?;

        save_asset_mapping_v1(
            deps.storage,
            &alt_asset_id_to_update.alt_asset_id,
            alt_asset_id_to_update.mapping,
        )?;
    }

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ATTRIBUTE_ACTION_VALUE)
        .to_ok()
}
