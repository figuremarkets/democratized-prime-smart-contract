//! Reduce `deficit_underlying` using either **`accrued_reserve`** (contract owner only) or **`bank`**
//! (any sender may attach lending-denom coins; excess refunded to the sender).

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_DEFICIT_UNDERLYING, ATTRIBUTE_SENDER,
};
use crate::model::error::{illegal_argument, illegal_state, invalid_funds, ContractError};
use crate::msg::execute::EliminateDeficitFunding;
use crate::storage::{get_contract_state_v1, set_reserve_state_v1};
use crate::utils::{update_reserve_indexes, validate_single_coin_denom, WithRates};
use cosmwasm_std::{ensure, BankMsg, Coin, DepsMut, Env, MessageInfo, Response, Uint128};
use democratized_prime_lib::common::assert_owner;

pub const ACTION: &str = "eliminate_deficit";
pub const ASSERT_OWNER_ERR: &str =
    "Only the contract owner may eliminate deficit using accrued_reserve";

/// Apply up to `max_underlying` from the chosen source toward `deficit_underlying` (partial OK).
pub fn eliminate_deficit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    funding: EliminateDeficitFunding,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;

    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;
    ensure!(
        reserve.deficit_underlying > 0,
        illegal_argument("Reserve has no deficit to eliminate")
    );

    let max = match &funding {
        EliminateDeficitFunding::AccruedReserve { max_underlying } => max_underlying.u128(),
        EliminateDeficitFunding::Bank { max_underlying } => max_underlying.u128(),
    };
    ensure!(max > 0, illegal_argument("max_underlying must be positive"));

    let mut res = Response::new().add_attribute(ATTRIBUTE_ACTION_NAME, ACTION);

    match funding {
        EliminateDeficitFunding::AccruedReserve { .. } => {
            assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
            ensure!(
                info.funds.is_empty(),
                invalid_funds("No funds accepted for accrued_reserve funding")
            );
            let clear = reserve
                .deficit_underlying
                .min(max)
                .min(reserve.accrued_reserve);
            ensure!(
                clear > 0,
                illegal_state("Nothing to clear from accrued_reserve")
            );
            reserve.deficit_underlying = reserve
                .deficit_underlying
                .checked_sub(clear)
                .ok_or_else(|| illegal_state("deficit_underlying underflow"))?;
            reserve.accrued_reserve = reserve
                .accrued_reserve
                .checked_sub(clear)
                .ok_or_else(|| illegal_state("accrued_reserve underflow"))?;
            res = res.add_attribute(ATTRIBUTE_AMOUNT, clear.to_string());
        }
        EliminateDeficitFunding::Bank { .. } => {
            let received =
                validate_single_coin_denom(&info, &contract.lending_denom, Uint128::one())?.u128();
            let applied = reserve.deficit_underlying.min(max).min(received);
            ensure!(applied > 0, illegal_state("Nothing applied toward deficit"));
            reserve.deficit_underlying = reserve
                .deficit_underlying
                .checked_sub(applied)
                .ok_or_else(|| illegal_state("deficit_underlying underflow"))?;
            res = res.add_attribute(ATTRIBUTE_AMOUNT, applied.to_string());
            let refund = received.saturating_sub(applied);
            if refund > 0 {
                res = res.add_message(BankMsg::Send {
                    to_address: info.sender.to_string(),
                    amount: vec![Coin {
                        denom: contract.lending_denom.name.clone(),
                        amount: Uint128::from(refund),
                    }],
                });
            }
        }
    }

    set_reserve_state_v1(deps.storage, &reserve)?;
    res.add_attribute(
        ATTRIBUTE_DEFICIT_UNDERLYING,
        reserve.deficit_underlying.to_string(),
    )
    .add_attribute(ATTRIBUTE_SENDER, info.sender.as_str())
    .attach_rates(&reserve, &contract.rate_params)
}
