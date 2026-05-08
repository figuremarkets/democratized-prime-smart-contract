use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_LENDER, ATTRIBUTE_RECIPIENT,
    ATTRIBUTE_SCALED_AMOUNT,
};
use crate::model::error::{illegal_argument, ContractError};
use crate::storage::{get_contract_state_v1, get_lender_require_commit_on_exit};
use crate::utils::{
    compute_effective_reserve, underlying_to_scaled_liquidity, validate_lender_attrs, WithRates,
};
use cosmwasm_std::{ensure, to_json_binary, Addr, DepsMut, Env, Response, Uint128, WasmMsg};
use cw20::Cw20ExecuteMsg;
use result_extensions::ResultExtensions;

pub const ACTION: &str = "transfer";
pub const ACTION_EXACT: &str = "transfer_exact";

/// Transfer/transfer_exact when repo token is sent via CW20 Receive. Forwards CW20 to recipient; optionally refunds excess to sender.
/// When sender has require_commit_on_exit set, transfer is disallowed entirely so they cannot move funds without re-committing via withdraw.
pub fn execute_transfer_cw20(
    deps: DepsMut,
    env: Env,
    sender: Addr,
    received_scaled: u128,
    recipient: String,
    amount_underlying: Option<Uint128>,
    exact: bool,
) -> Result<Response, ContractError> {
    ensure!(
        received_scaled > 0,
        illegal_argument("Transfer amount must be greater than zero")
    );
    let contract = get_contract_state_v1(deps.storage)?;
    let effective_require = get_lender_require_commit_on_exit(deps.storage, &sender)?;
    ensure!(
        !effective_require,
        illegal_argument(
            "Transfers are not allowed while commitment-on-exit is required; withdraw with commit_funds to return funds to the committed market first",
        )
    );
    let recipient_addr = deps.api.addr_validate(&recipient)?;
    ensure!(
        sender != recipient_addr,
        illegal_argument("Recipient must be different from sender")
    );

    let repo_token = contract.repo_token_addr()?;
    validate_lender_attrs(
        &deps.querier,
        recipient_addr.as_str(),
        &contract.lender_required_attrs,
    )?;
    let reserve = compute_effective_reserve(deps.storage, env.block.time, &contract.rate_params)?;

    let (scaled_to_send, attribute_amount, action_name) = if exact {
        (
            received_scaled,
            Uint128::from(received_scaled),
            ACTION_EXACT,
        )
    } else {
        let amount =
            amount_underlying.ok_or_else(|| illegal_argument("Transfer amount required"))?;
        ensure!(
            !amount.is_zero(),
            illegal_argument("Transfer amount must be greater than zero")
        );
        let scaled = underlying_to_scaled_liquidity(amount.u128(), reserve.liquidity_index)?;
        ensure!(
            received_scaled >= scaled,
            illegal_argument(format!(
                "Insufficient repo token sent: need {} (scaled) for {} underlying, got {}",
                scaled, amount, received_scaled
            ))
        );
        (scaled, amount, ACTION)
    };

    let mut res = Response::new()
        .add_message(WasmMsg::Execute {
            contract_addr: repo_token.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                recipient: recipient.clone(),
                amount: Uint128::from(scaled_to_send),
            })
            .map_err(ContractError::Std)?,
            funds: vec![],
        })
        .add_attribute(ATTRIBUTE_ACTION_NAME, action_name)
        .add_attribute(ATTRIBUTE_LENDER, sender.as_str())
        .add_attribute(ATTRIBUTE_RECIPIENT, recipient)
        .add_attribute(ATTRIBUTE_AMOUNT, attribute_amount.to_string())
        .add_attribute(ATTRIBUTE_SCALED_AMOUNT, scaled_to_send.to_string());

    if !exact && received_scaled > scaled_to_send {
        let refund = Uint128::new(received_scaled)
            .checked_sub(Uint128::new(scaled_to_send))?
            .u128();
        res = res.add_message(WasmMsg::Execute {
            contract_addr: repo_token.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                recipient: sender.to_string(),
                amount: Uint128::from(refund),
            })
            .map_err(ContractError::Std)?,
            funds: vec![],
        });
    }

    res = res.attach_rates(&reserve, &contract.rate_params)?;
    res.to_ok()
}
