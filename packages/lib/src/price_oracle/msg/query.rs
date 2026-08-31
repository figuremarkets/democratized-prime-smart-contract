use crate::price_oracle::model::{ContractStateV1, PriceMapResponse};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cw_ownable::cw_ownable_query;

#[cw_ownable_query]
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ContractStateV1)]
    GetState {},

    #[returns(PriceMapResponse)]
    GetPrices {
        prev_asset: Option<String>,
        limit: u32,
    },

    #[returns(PriceMapResponse)]
    GetPricesByAsset {
        assets: Vec<String>,
        /// When true, assets with no stored price are omitted from the map instead of
        /// erroring. Defaults to false (all-or-nothing).
        #[serde(default)]
        skip_missing: bool,
    },
}
