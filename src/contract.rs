use cosmwasm_std::{entry_point, DepsMut, Env, MessageInfo, Response, Addr};

use crate::{
    error::ContractError, msg::{ExecuteMsg, InstantiateMsg}, state::{
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

#[entry_point]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Test {} => Ok(exec::test_swap(info.sender))
    }
}

mod exec {
    use cosmwasm_std::CosmosMsg;
    use osmosis_std::types::{
        cosmos::base::v1beta1::Coin,
        osmosis::poolmanager::v1beta1::{
            MsgSwapExactAmountIn, SwapAmountInRoute
        }
    };

    use super::*;

    pub fn test_swap(sender: Addr) -> Response {
        let sender = sender.to_string();
        
        // Pool id from a testnet tx I did.
        let osmo_to_atom_pool_id: u64 = 367;

        // Denom I got from the same tx, also from the osmo testnet asset list.
        let atom_denom = 
            "ibc/9FF2B7A5F55038A7EE61F4FD6749D9A648B48E89830F2682B67B5DC158E2753C"
            .to_string();


        // TODO We can call `CalcOutAmtGivenIn` to get our amounts!
        let coin_in = Coin {
            denom: "uosmo".to_string(),
            amount: 1000.to_string()
        };

        let route = SwapAmountInRoute {
            pool_id: osmo_to_atom_pool_id,
            token_out_denom: atom_denom
        };

        // How do we know the actual pool contract? Is it somehow "native?".
        // We will see...
        let swap_msg: CosmosMsg = MsgSwapExactAmountIn {
            sender,
            routes: vec![route],
            token_in: Some(coin_in),
            token_out_min_amount: 69.to_string()
        }.into();

        Response::new().add_message(swap_msg)
    }
}
