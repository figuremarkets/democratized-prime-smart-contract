#[cfg(test)]
mod unit {
    use crate::constants::ATTRIBUTE_ACTION_NAME;
    use crate::contract::execute;
    use crate::execute::update_asset_prices::ASSERT_OWNER_ERR;
    use crate::model::error::ContractError;
    use crate::model::{PriceUpdateV1, PriceV1};
    use crate::msg::execute::ExecuteMsg;
    use crate::storage::{get_contract_state_v1, get_sorted_prices_v1};
    use crate::tests::constants::{
        ADMIN_ADDRESS, DENOM0, DENOM1, EPOCH_SECOND_JAN_01_2025, NON_ADMIN_ADDRESS,
    };
    use crate::tests::helpers::mock_env_with_timestamp;
    use crate::tests::helpers::{mock_dependencies, ContractStateV1Builder, PriceV1Builder};
    use cosmwasm_std::testing::message_info;
    use cosmwasm_std::{coin, Addr, Decimal256, Response, Timestamp};
    use democratized_prime_lib::common::{illegal_argument, invalid_funds};
    use result_extensions::ResultExtensions;
    use std::str::FromStr;

    #[test]
    fn not_admin_then_return_unauthorized_err() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(NON_ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![PriceUpdateV1 {
                asset: DENOM0.to_string(),
                usd: Decimal256::from_str("1234.567").unwrap(),
                as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025.clone())),
            }],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert!(matches!(
            result,
            Err(ContractError::NotAuthorizedError { message })
                if message == ASSERT_OWNER_ERR
        ));

        let prices = get_sorted_prices_v1(&deps.storage, None, 100).unwrap();
        assert_eq!(prices.len(), 0);

        // No change to contract state
        assert_eq!(
            contract_state,
            get_contract_state_v1(&deps.storage).unwrap(),
        );
    }

    #[test]
    fn update_asset_prices_rejected_with_funds() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![coin(1, "nhash")]);

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![PriceUpdateV1 {
                asset: DENOM0.to_string(),
                usd: Decimal256::from_str("1234.567").unwrap(),
                as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025.clone())),
            }],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(
            result,
            invalid_funds("No funds accepted for price oracle actions").to_err()
        );
    }

    #[test]
    fn current_prices_empty_then_save_prices_return_ok() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        // No prices set

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![
                PriceUpdateV1 {
                    asset: DENOM0.to_string(),
                    usd: Decimal256::from_str("1234.567").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025.clone())),
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::from_str("7654.321").unwrap(),
                    as_of: Some(Timestamp::from_seconds(
                        EPOCH_SECOND_JAN_01_2025.clone() + 1,
                    )),
                },
            ],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(
            result,
            Response::new()
                .add_attribute(ATTRIBUTE_ACTION_NAME, "set_asset_prices")
                .to_ok(),
        );

        let prices = get_sorted_prices_v1(&deps.storage, None, 100).unwrap();
        assert_eq!(prices.len(), 2);

        let p0 = prices.get(0).unwrap();
        assert_eq!(p0.0, DENOM0,);
        assert_eq!(
            p0.1,
            PriceV1 {
                price_usd: Decimal256::from_str("1234.567").unwrap(),
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025.clone(),
            },
        );

        let p1 = prices.get(1).unwrap();
        assert_eq!(p1.0, DENOM1,);
        assert_eq!(
            p1.1,
            PriceV1 {
                price_usd: Decimal256::from_str("7654.321").unwrap(),
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025.clone() + 1,
            },
        );

        // No change to contract state
        assert_eq!(
            contract_state,
            get_contract_state_v1(&deps.storage).unwrap(),
        );
    }

    #[test]
    fn empty_update_as_of_time_then_save_block_time_and_return_ok() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        // No prices set

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![
                PriceUpdateV1 {
                    asset: DENOM0.to_string(),
                    usd: Decimal256::from_str("1234.567").unwrap(),
                    as_of: None,
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::from_str("7654.321").unwrap(),
                    as_of: None,
                },
            ],
        };

        let result = execute(deps.as_mut(), env.clone(), info.clone(), msg);
        assert_eq!(
            result,
            Response::new()
                .add_attribute(ATTRIBUTE_ACTION_NAME, "set_asset_prices")
                .to_ok(),
        );

        let prices = get_sorted_prices_v1(&deps.storage, None, 100).unwrap();
        assert_eq!(prices.len(), 2);

        let p0 = prices.get(0).unwrap();
        assert_eq!(p0.0, DENOM0,);
        assert_eq!(
            p0.1,
            PriceV1 {
                price_usd: Decimal256::from_str("1234.567").unwrap(),
                // Block time
                as_of_epoch_second: env.block.time.seconds().clone(),
            },
        );

        let p1 = prices.get(1).unwrap();
        assert_eq!(p1.0, DENOM1,);
        assert_eq!(
            p1.1,
            PriceV1 {
                price_usd: Decimal256::from_str("7654.321").unwrap(),
                // Block time
                as_of_epoch_second: env.block.time.seconds().clone(),
            },
        );

        // No change to contract state
        assert_eq!(
            contract_state,
            get_contract_state_v1(&deps.storage).unwrap(),
        );
    }

    #[test]
    fn reject_if_an_asset_name_is_duplicated() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        let _initial_denom0_price = PriceV1Builder::new()
            .set_asset_id(DENOM0)
            .set_price_usd(&Decimal256::from_str("1234.000").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);
        let _initial_denom1_price = PriceV1Builder::new()
            .set_asset_id(DENOM1)
            .set_price_usd(&Decimal256::from_str("12.345").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![
                PriceUpdateV1 {
                    asset: DENOM0.to_string(),
                    usd: Decimal256::from_str("44.5566").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025.clone())),
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::from_str("1234.567").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025.clone())),
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::from_str("7654.321").unwrap(),
                    as_of: Some(Timestamp::from_seconds(
                        EPOCH_SECOND_JAN_01_2025.clone() + 1,
                    )),
                },
            ],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(
            result,
            illegal_argument(format!("Duplicate name: {DENOM1}")).to_err()
        );
    }

    #[test]
    fn reject_if_asset_name_is_empty() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        let _initial_denom0_price = PriceV1Builder::new()
            .set_asset_id(DENOM0)
            .set_price_usd(&Decimal256::from_str("1234.000").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);
        let _initial_denom1_price = PriceV1Builder::new()
            .set_asset_id(DENOM1)
            .set_price_usd(&Decimal256::from_str("12.345").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![
                PriceUpdateV1 {
                    asset: "".to_string(),
                    usd: Decimal256::from_str("1234.567").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025.clone())),
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::from_str("7654.321").unwrap(),
                    as_of: Some(Timestamp::from_seconds(
                        EPOCH_SECOND_JAN_01_2025.clone() + 1,
                    )),
                },
            ],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(
            result,
            illegal_argument("Asset name cannot be empty").to_err()
        )
    }

    #[test]
    fn reject_if_asset_price_is_zero() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        let _initial_denom0_price = PriceV1Builder::new()
            .set_asset_id(DENOM0)
            .set_price_usd(&Decimal256::from_str("1234.000").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);
        let _initial_denom1_price = PriceV1Builder::new()
            .set_asset_id(DENOM1)
            .set_price_usd(&Decimal256::from_str("12.345").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![
                PriceUpdateV1 {
                    asset: DENOM0.to_string(),
                    usd: Decimal256::from_str("1234.567").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025.clone())),
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::zero(),
                    as_of: Some(Timestamp::from_seconds(
                        EPOCH_SECOND_JAN_01_2025.clone() + 1,
                    )),
                },
            ],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(
            result,
            illegal_argument(format!("{DENOM1}: asset price must be > 0")).to_err()
        )
    }

    #[test]
    fn reject_if_price_data_timestamp_is_too_old() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        let _initial_denom0_price = PriceV1Builder::new()
            .set_asset_id(DENOM0)
            .set_price_usd(&Decimal256::from_str("1234.000").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);
        let _initial_denom1_price = PriceV1Builder::new()
            .set_asset_id(DENOM1)
            .set_price_usd(&Decimal256::from_str("12.345").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![
                PriceUpdateV1 {
                    asset: DENOM0.to_string(),
                    usd: Decimal256::from_str("1234.567").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025).minus_days(60)),
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::from_str("7654.321").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025 + 1)),
                },
            ],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(result, illegal_argument("Price timestamp too old").to_err());
    }

    #[test]
    fn reject_if_price_data_timestamp_is_too_far_into_the_future() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        let _initial_denom0_price = PriceV1Builder::new()
            .set_asset_id(DENOM0)
            .set_price_usd(&Decimal256::from_str("1234.000").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);
        let _initial_denom1_price = PriceV1Builder::new()
            .set_asset_id(DENOM1)
            .set_price_usd(&Decimal256::from_str("12.345").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![
                PriceUpdateV1 {
                    asset: DENOM0.to_string(),
                    usd: Decimal256::from_str("1234.567").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025).plus_days(60)),
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::from_str("7654.321").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025 + 1)),
                },
            ],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(
            result,
            illegal_argument("Price timestamp too far into the future").to_err()
        );
    }

    #[test]
    fn current_prices_exist_then_update_prices_return_ok() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        let _initial_denom0_price = PriceV1Builder::new()
            .set_asset_id(DENOM0)
            .set_price_usd(&Decimal256::from_str("1234.000").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);
        let _initial_denom1_price = PriceV1Builder::new()
            .set_asset_id(DENOM1)
            .set_price_usd(&Decimal256::from_str("12.345").unwrap())
            .set_as_of_time(EPOCH_SECOND_JAN_01_2025.clone() - 100)
            .build_and_store(deps.as_mut().storage);

        let msg = ExecuteMsg::UpdateAssetPrices {
            prices: vec![
                PriceUpdateV1 {
                    asset: DENOM0.to_string(),
                    usd: Decimal256::from_str("1234.567").unwrap(),
                    as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025.clone())),
                },
                PriceUpdateV1 {
                    asset: DENOM1.to_string(),
                    usd: Decimal256::from_str("7654.321").unwrap(),
                    as_of: Some(Timestamp::from_seconds(
                        EPOCH_SECOND_JAN_01_2025.clone() + 1,
                    )),
                },
            ],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(
            result,
            Response::new()
                .add_attribute(ATTRIBUTE_ACTION_NAME, "set_asset_prices")
                .to_ok(),
        );

        // Updated prices
        let prices = get_sorted_prices_v1(&deps.storage, None, 100).unwrap();
        assert_eq!(prices.len(), 2);

        let p0 = prices.get(0).unwrap();
        assert_eq!(p0.0, DENOM0,);
        assert_eq!(
            p0.1,
            PriceV1 {
                price_usd: Decimal256::from_str("1234.567").unwrap(),
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025.clone(),
            },
        );

        let p1 = prices.get(1).unwrap();
        assert_eq!(p1.0, DENOM1,);
        assert_eq!(
            p1.1,
            PriceV1 {
                price_usd: Decimal256::from_str("7654.321").unwrap(),
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025.clone() + 1,
            },
        );

        // No change to contract state
        assert_eq!(
            contract_state,
            get_contract_state_v1(&deps.storage).unwrap(),
        );
    }
}
