#[cfg(test)]
mod unit {
    use crate::constants::ATTRIBUTE_ACTION_NAME;
    use crate::contract::instantiate;
    use crate::msg::instantiate::InstantiateMsg;
    use crate::storage::contract_state::get_contract_state_v1;
    use crate::storage::prices::get_sorted_prices_v1;
    use crate::tests::constants::{ADMIN_ADDRESS, NON_ADMIN_ADDRESS};
    use crate::tests::helpers::mock_dependencies;
    use cosmwasm_std::testing::{message_info, mock_env};
    use cosmwasm_std::{Addr, Response};
    use cw2::{get_contract_version, ContractVersion};
    use cw_ownable::get_ownership;
    use democratized_prime_lib::price_oracle::model::ContractStateV1;
    use result_extensions::ResultExtensions;

    #[test]
    fn instantiate_then_set_state_and_return_ok() {
        let mut deps = mock_dependencies(&vec![]);
        let sender_addr = Addr::unchecked(NON_ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let msg = InstantiateMsg {
            owner: ADMIN_ADDRESS.to_string(),
        };
        let result = instantiate(deps.as_mut(), mock_env(), info.clone(), msg);
        assert_eq!(
            result,
            Response::new()
                .add_attribute(ATTRIBUTE_ACTION_NAME, "instantiate")
                .to_ok(),
        );

        // No price data
        let prices = get_sorted_prices_v1(&deps.storage, None, 100).unwrap();
        assert_eq!(prices.len(), 0);

        assert_eq!(
            ContractStateV1 {},
            get_contract_state_v1(&deps.storage).unwrap(),
        );

        let o = get_ownership(&deps.storage).unwrap();
        assert_eq!(o.owner, Some(Addr::unchecked(ADMIN_ADDRESS)));

        // Set contract version
        assert_eq!(
            ContractVersion {
                contract: "democratized_prime_price_oracle".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            get_contract_version(&deps.storage).unwrap(),
        );
    }
}
