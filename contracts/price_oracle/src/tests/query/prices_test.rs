#[cfg(test)]
#[allow(deprecated)]
mod query_prices_by_asset_unit {
    use crate::constants::DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS;
    use crate::model::error::QueryError;
    use crate::query::query_prices_by_assets;
    use crate::tests::constants::EPOCH_SECOND_JAN_01_2025;
    use crate::tests::helpers::{mock_dependencies, AssetMappingV1Builder, PriceV1Builder};
    use cosmwasm_std::{from_json, Decimal256, Storage};
    use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
    use result_extensions::ResultExtensions;
    use std::collections::HashMap;
    use std::str::FromStr;

    fn setup_mapping(store: &mut dyn Storage, staleness_threshold_seconds: u32) {
        AssetMappingV1Builder::new()
            .set_asset_id("BTC")
            .set_alt_asset_id("nbtc.figure.se")
            .set_precision(9)
            .set_staleness_threshold_seconds(staleness_threshold_seconds)
            .build_and_store(store);
    }

    fn setup_price(store: &mut dyn Storage, as_of_time: u64) {
        PriceV1Builder::new()
            .set_asset_id("BTC")
            .set_as_of_time(as_of_time)
            .set_price_usd(&Decimal256::from_str("100000.123").unwrap())
            .build_and_store(store);
    }

    #[test]
    fn get_by_alt_asset_id_price_returns_price_map() {
        let mut deps = mock_dependencies(&[]);

        setup_mapping(
            deps.as_mut().storage,
            DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
        );
        // price is 10 seconds old:
        setup_price(deps.as_mut().storage, EPOCH_SECOND_JAN_01_2025 - 10);

        let result =
            query_prices_by_assets(&deps.storage, vec![String::from("nbtc.figure.se")], false);
        let result_body: PriceMapResponse = from_json(result.unwrap()).unwrap();

        let mut expected: PriceMapResponse = HashMap::new();
        expected.insert(
            "nbtc.figure.se".to_string(),
            AssetPriceResponseV1 {
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025 - 10,
                price_usd: Decimal256::from_str("0.000100000123").unwrap(),
                display_price_usd: Decimal256::from_str("100000.123").unwrap(),
                precision: 9,
                expiration_epoch_seconds: (EPOCH_SECOND_JAN_01_2025 - 10)
                    + (DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS as u64),
            },
        );
        assert_eq!(result_body, expected)
    }

    #[test]
    fn get_by_display_asset_id_price_returns_price_map() {
        let mut deps = mock_dependencies(&[]);

        setup_mapping(
            deps.as_mut().storage,
            DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
        );
        // price is 10 seconds old:
        setup_price(deps.as_mut().storage, EPOCH_SECOND_JAN_01_2025 - 10);

        let result = query_prices_by_assets(&deps.storage, vec![String::from("BTC")], false);
        let result_body: PriceMapResponse = from_json(result.unwrap()).unwrap();

        let mut expected: PriceMapResponse = HashMap::new();
        expected.insert(
            "BTC".to_string(),
            AssetPriceResponseV1 {
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025 - 10,
                price_usd: Decimal256::from_str("100000.123").unwrap(),
                display_price_usd: Decimal256::from_str("100000.123").unwrap(),
                precision: 0,
                expiration_epoch_seconds: (EPOCH_SECOND_JAN_01_2025 - 10)
                    + (DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS as u64),
            },
        );
        assert_eq!(result_body, expected,)
    }

    #[test]
    fn mapping_found_but_price_not_found_return_error() {
        let mut deps = mock_dependencies(&[]);

        setup_mapping(
            deps.as_mut().storage,
            DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
        );
        // No price for BTC

        let result =
            query_prices_by_assets(&deps.storage, vec![String::from("nbtc.figure.se")], false);
        assert_eq!(
            result,
            QueryError::NotFoundError {
                message: "BTC".to_owned()
            }
            .to_err()
        );
    }

    #[test]
    fn mapping_not_found_and_price_not_found_return_error() {
        let deps = mock_dependencies(&[]);

        // No mapping for nbtc.figure.se
        // No price for BTC

        let result = query_prices_by_assets(
            &deps.storage,
            vec![String::from("nbtc.figure.se"), String::from("BTC")],
            false,
        );
        assert_eq!(
            result,
            QueryError::NotFoundError {
                message: "nbtc.figure.se".to_owned()
            }
            .to_err()
        );
    }

    #[test]
    fn skip_missing_omits_assets_without_a_stored_price() {
        let mut deps = mock_dependencies(&[]);

        setup_mapping(
            deps.as_mut().storage,
            DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
        );
        setup_price(deps.as_mut().storage, EPOCH_SECOND_JAN_01_2025 - 10);

        let result = query_prices_by_assets(
            &deps.storage,
            vec![
                String::from("nbtc.figure.se"),
                String::from("missing.asset"),
            ],
            true,
        );
        let result_body: PriceMapResponse = from_json(result.unwrap()).unwrap();

        assert_eq!(result_body.len(), 1);
        assert!(result_body.contains_key("nbtc.figure.se"));
        assert!(!result_body.contains_key("missing.asset"));
    }

    #[test]
    fn skip_missing_returns_empty_map_when_no_requested_asset_has_a_price() {
        let deps = mock_dependencies(&[]);

        let result = query_prices_by_assets(
            &deps.storage,
            vec![String::from("nbtc.figure.se"), String::from("BTC")],
            true,
        );
        let result_body: PriceMapResponse = from_json(result.unwrap()).unwrap();
        assert!(result_body.is_empty());
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod scale_truncation {
    use crate::constants::DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS;
    use crate::query::query_prices_by_assets;
    use crate::tests::constants::EPOCH_SECOND_JAN_01_2025;
    use crate::tests::helpers::{mock_dependencies, AssetMappingV1Builder, PriceV1Builder};
    use cosmwasm_std::{from_json, Decimal256};
    use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
    use std::str::FromStr;

    #[test]
    fn get_by_alt_asset_emits_display_fields_when_scaled_price_is_zero() {
        let mut deps = mock_dependencies(&[]);
        AssetMappingV1Builder::new()
            .set_asset_id("ETH")
            .set_alt_asset_id("neth.figure.se")
            .set_precision(18)
            .set_staleness_threshold_seconds(DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS)
            .build_and_store(deps.as_mut().storage);
        PriceV1Builder::new()
            .set_asset_id("ETH")
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025 - 10)
            .set_price_usd(&Decimal256::from_str("0.5").unwrap())
            .build_and_store(deps.as_mut().storage);

        let result_body: PriceMapResponse = from_json(
            query_prices_by_assets(&deps.storage, vec![String::from("neth.figure.se")]).unwrap(),
        )
        .unwrap();
        let price = result_body.get("neth.figure.se").unwrap();
        assert_eq!(
            price,
            &AssetPriceResponseV1 {
                price_usd: Decimal256::zero(),
                display_price_usd: Decimal256::from_str("0.5").unwrap(),
                precision: 18,
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025 - 10,
                expiration_epoch_seconds: (EPOCH_SECOND_JAN_01_2025 - 10)
                    + (DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS as u64),
            }
        );
        assert_eq!(
            price.value_usd(1_000_000_000_000_000_000).unwrap(),
            Decimal256::from_str("0.5").unwrap()
        );
    }
}
