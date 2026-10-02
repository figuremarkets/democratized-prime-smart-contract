//! One directed-sale offer per borrower. Missing key means no offer (including after cancel or fill).

use crate::model::directed_sale::DirectedSaleOfferV1;
use crate::model::error::ContractError;
use cosmwasm_std::Storage;
use cw_storage_plus::Map;

const STORAGE_KEY_DIRECTED_SALE_OFFER: &str = "dso1";
const DIRECTED_SALE_OFFER: Map<String, DirectedSaleOfferV1> =
    Map::new(STORAGE_KEY_DIRECTED_SALE_OFFER);

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
