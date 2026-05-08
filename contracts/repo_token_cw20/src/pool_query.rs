//! Minimal types to parse the pool's GetReserve response.
//! The pool returns more fields (borrow_index, totals, rates, utilization); we only need
//! `reserve.liquidity_index` for scaled→underlying. Serde ignores unknown fields when deserializing.

use cosmwasm_std::{Addr, Decimal256, QuerierWrapper};
use serde::Deserialize;
use serde::Serialize;
use std::str::FromStr;

use crate::error::{illegal_argument, ContractError};

#[derive(Clone, Serialize, Deserialize)]
pub struct PoolReserveResponse {
    pub reserve: PoolReserveState,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PoolReserveState {
    pub liquidity_index: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum PoolQueryMsg {
    GetReserve {},
}

/// Query the pool contract for current liquidity index (for scaled -> underlying conversion).
pub fn query_liquidity_index(
    querier: &QuerierWrapper,
    pool_address: &Addr,
) -> Result<Decimal256, ContractError> {
    let res = querier
        .query_wasm_smart::<PoolReserveResponse>(pool_address, &PoolQueryMsg::GetReserve {})?;
    Decimal256::from_str(&res.reserve.liquidity_index)
        .map_err(|e| illegal_argument(format!("invalid liquidity_index: {}", e)))
}
