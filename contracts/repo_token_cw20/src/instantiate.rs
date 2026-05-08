use cosmwasm_std::{Addr, DepsMut, Env, MessageInfo, Response, Uint128};

use crate::error::illegal_argument;
use cw2::set_contract_version;
use cw_ownable::initialize_owner;
use democratized_prime_lib::repo_token::validate_repo_token_meta;

use crate::constants::{CONTRACT_NAME, CONTRACT_VERSION};
use crate::error::ContractError;
use crate::msg::InstantiateMsg;
use crate::state::{Config, TokenInfo, CONFIG, TOKEN_INFO};

pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    validate_repo_token_meta(&msg.name, &msg.symbol, msg.decimals).map_err(illegal_argument)?;
    let owner: Addr = deps.api.addr_validate(msg.owner.as_str())?;
    let minter: Addr = deps.api.addr_validate(msg.minter.as_str())?;
    let pool_address: Option<Addr> = match msg.pool_address {
        Some(p) => Some(deps.api.addr_validate(p.as_str())?),
        None => None,
    };
    CONFIG.save(
        deps.storage,
        &Config {
            minter,
            pool_address,
        },
    )?;
    TOKEN_INFO.save(
        deps.storage,
        &TokenInfo {
            name: msg.name,
            symbol: msg.symbol,
            decimals: msg.decimals,
            total_supply: Uint128::zero(),
        },
    )?;
    initialize_owner(deps.storage, deps.api, Some(owner.as_str()))?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}
