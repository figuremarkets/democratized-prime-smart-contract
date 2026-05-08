use crate::constants::ATTRIBUTE_ACTION_NAME;
use crate::model::{error::ContractError, PriceUpdateV1};
use crate::storage::save_usd_price_v1;
use crate::utils::validate_name_uniqueness;
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::assert_owner;
use result_extensions::ResultExtensions;

pub const ATTRIBUTE_ACTION_VALUE: &str = "set_asset_prices";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may update asset prices";

/// Attempt to update asset prices.
///
/// # Arguments
///
/// * `price_updates` - Updates asset prices.
pub fn try_update_asset_prices(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    price_updates: Vec<PriceUpdateV1>,
) -> Result<Response, ContractError> {
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;

    // Validate unique asset IDs:
    let asset_ids = price_updates
        .iter()
        .map(|update| update.asset.clone())
        .collect::<Vec<_>>();

    validate_name_uniqueness(&asset_ids)?;

    for price_update in price_updates {
        price_update.validate(env.block.time)?;

        save_usd_price_v1(
            deps.storage,
            price_update.asset.clone(),
            &(&env, &price_update).into(),
        )?;
    }

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ATTRIBUTE_ACTION_VALUE)
        .to_ok()
}
