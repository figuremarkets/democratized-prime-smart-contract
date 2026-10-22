mod add_collateral_tests;
mod borrow_tests;
mod directed_sale_tests;
mod eliminate_deficit_tests;
mod execute_msg_json_tests;
mod lend_rounding_audit_regression;
mod lend_tests;
mod liquidate_tests;
mod operational_state_tests;
mod remove_collateral_tests;
mod repay_tests;
mod set_borrower_require_commit_tests;
mod set_borrower_required_attrs_tests;
mod set_lender_require_commit_tests;
mod set_lender_required_attrs_tests;
mod socialize_deficit_tests;
mod transfer_tests;
mod update_contract_config_tests;
mod update_rate_params_tests;
mod update_supported_collateral_tests;
mod withdraw_reserve_tests;
mod withdraw_tests;

use crate::contract::execute;
use crate::msg::ExecuteMsg;
use crate::tests::query::common::{CUSTODIAN, OWNER};
use crate::tests::types::TestDeps;
use cosmwasm_std::testing::message_info;
use cosmwasm_std::{Addr, Env};

fn set_commit_market_id(deps: &mut TestDeps, env: &Env) {
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            max_liquidation_staleness_seconds: None,
            liquidation_access: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
            custodian: None,
            liquidator: None,
        },
    )
    .expect("set commit_market_id");
}

fn mark_borrower_require_commit(deps: &mut TestDeps, env: &Env, borrower: &Addr) {
    set_commit_market_id(deps, env);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: borrower.to_string(),
            require: Some(true),
        },
    )
    .expect("set borrower require commit");
}
