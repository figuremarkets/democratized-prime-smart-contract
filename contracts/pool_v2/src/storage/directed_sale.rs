//! One directed-sale offer per borrower. Missing offer key means no outstanding invitation
//! (including after cancel or fill). Version is a separate nonce and is never cleared by fill.

use crate::model::directed_sale::DirectedSaleOfferV1;
use crate::model::error::ContractError;
use cosmwasm_std::Storage;
use cw_storage_plus::Map;

const STORAGE_KEY_DIRECTED_SALE_OFFER: &str = "dso1";
const STORAGE_KEY_DIRECTED_SALE_VERSION: &str = "dsv1";
const DIRECTED_SALE_OFFER: Map<String, DirectedSaleOfferV1> =
    Map::new(STORAGE_KEY_DIRECTED_SALE_OFFER);
const DIRECTED_SALE_VERSION: Map<String, u64> = Map::new(STORAGE_KEY_DIRECTED_SALE_VERSION);

pub fn get_directed_sale_offer(
    store: &dyn Storage,
    borrower: &str,
) -> Result<Option<DirectedSaleOfferV1>, ContractError> {
    DIRECTED_SALE_OFFER
        .may_load(store, borrower.to_string())
        .map_err(ContractError::Std)
}

pub fn set_directed_sale_offer(
    store: &mut dyn Storage,
    borrower: &str,
    offer: &DirectedSaleOfferV1,
) -> Result<(), ContractError> {
    DIRECTED_SALE_OFFER
        .save(store, borrower.to_string(), offer)
        .map_err(ContractError::Std)
}

pub fn clear_directed_sale_offer(
    store: &mut dyn Storage,
    borrower: &str,
) -> Result<(), ContractError> {
    DIRECTED_SALE_OFFER.remove(store, borrower.to_string());
    Ok(())
}

/// Last consumed directed-sale nonce for this borrower. `0` if they have never mutated an offer.
pub fn get_directed_sale_version(
    store: &dyn Storage,
    borrower: &str,
) -> Result<u64, ContractError> {
    DIRECTED_SALE_VERSION
        .may_load(store, borrower.to_string())
        .map_err(ContractError::Std)
        .map(|v| v.unwrap_or(0))
}

pub fn set_directed_sale_version(
    store: &mut dyn Storage,
    borrower: &str,
    version: u64,
) -> Result<(), ContractError> {
    DIRECTED_SALE_VERSION
        .save(store, borrower.to_string(), &version)
        .map_err(ContractError::Std)
}
