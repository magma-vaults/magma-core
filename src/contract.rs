use cosmwasm_std::{entry_point, DepsMut, Env, MessageInfo, Response};

use crate::{
    error::ContractError, msg::InstantiateMsg, state::{
        VaultRebalancer, VaultState, VAULT_STATE
    }
};


#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg
) -> Result<Response, ContractError> {

    let admin = if let Some(addr) = &msg.admin {
        Some(deps.api.addr_validate(addr)?)
    } else { 
        None 
    };

    let vault_state = VaultState {
        pool: deps.api.addr_validate(&msg.pool)?,
        admin,
        rebalancer: msg.rebalancer.addr_validate(deps.as_ref())?,
        config: msg.config
    };

    if let None = vault_state.admin {
        match vault_state.rebalancer {
            VaultRebalancer::Anyone {} => Ok(()),
            _ => Err(ContractError::InvalidConfig {}),
        }?
    }
    
    VAULT_STATE.save(deps.storage, &vault_state)?;
    Ok(Response::new())
}

