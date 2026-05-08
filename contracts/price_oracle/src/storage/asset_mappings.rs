use crate::model::asset_mapping::AssetMappingV1;
use crate::model::error::ContractError;
use cosmwasm_std::Storage;
use cw_storage_plus::Map;
use result_extensions::ResultExtensions;

const STORAGE_KEY_ASSET_MAPPINGS_V1: &str = "am1";
const ASSET_MAPPINGS_V1: Map<String, AssetMappingV1> = Map::new(STORAGE_KEY_ASSET_MAPPINGS_V1);

/// Get display asset/metadata for an alt_asset_id
pub fn try_get_asset_mapping_v1(
    store: &dyn Storage,
    alt_asset_id: &str,
) -> Result<Option<AssetMappingV1>, ContractError> {
    ASSET_MAPPINGS_V1
        .may_load(store, alt_asset_id.to_owned())
        .map_err(ContractError::Std)
}

/// Check if there is a mapping of an alt_asset_id to a display asset (e.g. nbtc.figure.se -> BTC)
/// If no mapping found, return the requested asset_id with metadata containing a precision of 0
pub fn get_or_default_asset_mapping_v1(
    store: &dyn Storage,
    alt_asset_id: &str,
) -> Result<(String, AssetMappingV1), ContractError> {
    try_get_asset_mapping_v1(store, alt_asset_id)?
        .map_or(
            (
                alt_asset_id.to_owned(),
                AssetMappingV1::default(alt_asset_id.to_owned()),
            ),
            |am| (alt_asset_id.to_owned(), am),
        )
        .to_ok()
}

/// Save mapping of alt_asset_id -> display asset/metadata
pub fn save_asset_mapping_v1(
    store: &mut dyn Storage,
    alt_asset_id: &str,
    asset_id: AssetMappingV1,
) -> Result<(), ContractError> {
    ASSET_MAPPINGS_V1
        .save(store, alt_asset_id.to_owned(), &asset_id)
        .map_err(ContractError::Std)
}

/// Remove mapping of alt_asset_id -> display asset
pub fn remove_asset_mapping_v1(store: &mut dyn Storage, alt_asset_id: String) {
    ASSET_MAPPINGS_V1.remove(store, alt_asset_id);
}
