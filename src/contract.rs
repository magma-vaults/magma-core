use cosmwasm_std::{entry_point, DepsMut, Env, MessageInfo, Response, StdError };

use crate::{error::ContractError, msg::InstantiateMsg, state::{VaultManager, VaultRebalancer, VaultState, VAULT_STATE}};


#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg
) -> Result<Response, ContractError> {

    if env.block.height % 2 == 0 {
        return Err(ContractError::Std(StdError::generic_err("oops")));
    }

    let vault_state = VaultState {
        pool: deps.api.addr_validate(&msg.pool)?,
        manager: msg.manager,
        rebalancer: msg.rebalancer.addr_validate(deps.as_ref())?,
        config: msg.config
    };

    if let VaultManager::None {} = vault_state.manager {
        match vault_state.rebalancer {
            VaultRebalancer::Anyone {} => Ok(()),
            _ => Err(ContractError::InvalidConfig {})
        }?
    }

    VAULT_STATE.save(deps.storage, &vault_state);
    Ok(Response::new())
}

