use crate::model::asset_mapping::SaveAssetMappingRequestV1;
use crate::model::price::PriceUpdateV1;
use cosmwasm_schema::cw_serde;
use cw_ownable::cw_ownable_execute;

#[cw_ownable_execute]
#[cw_serde]
pub enum ExecuteMsg {
    UpdateAssetPrices {
        prices: Vec<PriceUpdateV1>,
    },

    UpdateAssetMappings {
        to_update: Vec<SaveAssetMappingRequestV1>,
        to_remove: Vec<String>,
    },
}
