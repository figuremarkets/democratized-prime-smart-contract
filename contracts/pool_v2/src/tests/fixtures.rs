//! Shared [`democratized_prime_lib::price_oracle::model::AssetPriceResponseV1`] fixtures for tests.
//! Expiration/`as_of` are derived from [`cosmwasm_std::Timestamp`] (typically `env.block.time` from [`cosmwasm_std::testing::mock_env`]).

use cosmwasm_std::{Decimal256, Timestamp};
use democratized_prime_lib::price_oracle::model::AssetPriceResponseV1;

pub fn fresh_oracle_price(price_usd: Decimal256, block_time: Timestamp) -> AssetPriceResponseV1 {
    let s = block_time.seconds();
    AssetPriceResponseV1::new(price_usd, s, s.saturating_add(1))
}

pub fn stale_oracle_price(price_usd: Decimal256, block_time: Timestamp) -> AssetPriceResponseV1 {
    let s = block_time.seconds();
    AssetPriceResponseV1::new(price_usd, s, s.saturating_sub(1))
}

/// Stored price whose expiration is `expired_for_secs` before `block_time` (last-known age).
pub fn oracle_price_expired_for(
    price_usd: Decimal256,
    block_time: Timestamp,
    expired_for_secs: u64,
) -> AssetPriceResponseV1 {
    let expiration = block_time.seconds().saturating_sub(expired_for_secs);
    let as_of = expiration.saturating_sub(1);
    AssetPriceResponseV1::new(price_usd, as_of, expiration)
}
