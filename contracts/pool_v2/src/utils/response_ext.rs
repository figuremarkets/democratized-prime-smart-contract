//! Extension traits for [`cosmwasm_std::Response`]: helpers that attach standard attributes or events.

use crate::constants::{
    ATTRIBUTE_BORROW_INDEX, ATTRIBUTE_BORROW_RATE, ATTRIBUTE_LEND_RATE, ATTRIBUTE_LIQUIDITY_INDEX,
    ATTRIBUTE_UTILIZATION,
};
use crate::model::error::ContractError;
use crate::model::{RateParamsV1, ReserveStateV1};
use crate::utils::rates::lend_borrow_rate_attribute_values;
use cosmwasm_std::Response;

pub trait WithRates {
    fn attach_rates(
        self,
        reserve: &ReserveStateV1,
        rate_params: &RateParamsV1,
    ) -> Result<Self, ContractError>
    where
        Self: Sized;
}

impl WithRates for Response {
    fn attach_rates(
        self,
        reserve: &ReserveStateV1,
        rate_params: &RateParamsV1,
    ) -> Result<Self, ContractError> {
        let (lend_s, borrow_s, li_s, bi_s, util_s) =
            lend_borrow_rate_attribute_values(reserve, rate_params)?;
        Ok(self
            .add_attribute(ATTRIBUTE_LEND_RATE, lend_s)
            .add_attribute(ATTRIBUTE_BORROW_RATE, borrow_s)
            .add_attribute(ATTRIBUTE_LIQUIDITY_INDEX, li_s)
            .add_attribute(ATTRIBUTE_BORROW_INDEX, bi_s)
            .add_attribute(ATTRIBUTE_UTILIZATION, util_s))
    }
}
