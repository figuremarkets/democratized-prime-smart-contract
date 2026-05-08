use cosmwasm_std::testing::message_info;
use cosmwasm_std::{Addr, Coin, MessageInfo};

use cosmwasm_std::testing::{mock_env, MockApi, MockStorage};
use cosmwasm_std::{Empty, Env, OwnedDeps, Timestamp};
use provwasm_mocks::{mock_provenance_dependencies, MockProvenanceQuerier};

pub fn mock_env_with_timestamp(time: Timestamp) -> Env {
    let mut env = mock_env();
    env.block.time = time;
    env
}

/// Setup `deps` with initial contract balances
pub fn mock_dependencies(
    contract_balances: &[Coin],
) -> OwnedDeps<MockStorage, MockApi, MockProvenanceQuerier, Empty> {
    let mut deps = mock_provenance_dependencies();

    deps.querier
        .mock_querier
        .bank
        .update_balance(&mock_env().contract.address, contract_balances.to_vec());

    deps.api = deps.api.with_prefix("tp");

    deps.into()
}

#[allow(dead_code)]
pub fn mock_info(sender: &str, funds: &[Coin]) -> MessageInfo {
    message_info(&Addr::unchecked(sender), funds)
}

#[allow(dead_code)]
pub fn mock_dependencies_builder() -> MockDependenciesBuilder {
    MockDependenciesBuilder::new()
}

pub struct MockDependenciesBuilder {
    deps: OwnedDeps<MockStorage, MockApi, MockProvenanceQuerier, Empty>,
}

impl MockDependenciesBuilder {
    pub fn new() -> Self {
        let mut deps = mock_provenance_dependencies();
        deps.api = deps.api.with_prefix("tp");

        let instance = Self { deps };

        instance
    }

    #[allow(dead_code)]
    pub fn set_bank_balance(mut self, address: &Addr, balance: &Vec<Coin>) -> Self {
        self.deps
            .querier
            .mock_querier
            .bank
            .update_balance(address, balance.to_vec());
        self
    }

    #[allow(dead_code)]
    pub fn build(self) -> OwnedDeps<MockStorage, MockApi, MockProvenanceQuerier, Empty> {
        self.deps.into()
    }
}
