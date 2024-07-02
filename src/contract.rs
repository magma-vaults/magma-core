use cosmwasm_std::{entry_point, to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult};

use crate::{
    error::ContractError, msg::{ExecuteMsg, InstantiateMsg}, state::{
        VaultRebalancer, VAULT_STATE
    }
};


#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg
) -> Result<Response, ContractError> {

    let state = msg
        .validate(deps.as_ref())
        .ok_or(ContractError::InvalidConfig {})?;

    if let None = state.vault_management_config.admin {
        match state.vault_management_config.rebalancer {
            VaultRebalancer::Anyone {} => Ok(()),
            _ => Err(ContractError::InvalidConfig {}),
        }?
    }

    VAULT_STATE.save(deps.storage, &state)?;
    Ok(Response::new())
}

#[entry_point]
pub fn query(_deps: Deps, _env: Env, _msg: Empty) -> StdResult<Binary> {
    to_json_binary("hi")
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Test {} => Ok(exec::test_swap(env, deps.as_ref())?)
    }
}

mod exec {

    use osmosis_std::types::{
        cosmos::base::v1beta1::Coin, osmosis::{
            gamm::v1beta1::MsgSwapExactAmountIn, poolmanager::v1beta1::SwapAmountInRoute
        }};

    use super::*;

    pub fn test_swap(env: Env, _deps: Deps) -> Result<Response, ContractError> {

        let sender = env.contract.address.to_string();
        
        // Pool id from a testnet tx I did.
        let osmo_to_atom_pool_id: u64 = 367;

        // Denom I got from the same tx, also from the osmo testnet asset list.
        let atom_denom = 
            "ibc/9FF2B7A5F55038A7EE61F4FD6749D9A648B48E89830F2682B67B5DC158E2753C"
            .to_string();

        // TODO We can call `CalcOutAmtGivenIn` to get our amounts! Or a querier
        // in general!
        let coin_in = Coin {
            denom: "uosmo".to_string(),
            amount: 1000.to_string()
        };

        let route = SwapAmountInRoute {
            pool_id: osmo_to_atom_pool_id,
            token_out_denom: atom_denom
        };

        let swap_msg = MsgSwapExactAmountIn {
            sender,
            routes: vec![route],
            token_in: Some(coin_in),
            token_out_min_amount: 69.to_string()
        };

        Ok(Response::new().add_message(swap_msg))
    }
}

#[cfg(test)]
mod test {
    use cw_multi_test::{App, ContractWrapper, Executor};

    use crate::{msg::VaultRebalancerInstantiationMsg, state::VaultParametersConfig};

    use super::*;

    // #[test]
    // fn basic_instantiation() {
    //     let mut app = App::default();
    //     let code = ContractWrapper::new(execute, instantiate, query);
    //     let code_id = app.store_code(Box::new(code));

    //     let owner = app.api().addr_make("owner");
    //     let _addr = app.instantiate_contract(
    //         code_id,
    //         owner.clone(),
    //         &InstantiateMsg {
    //             pool: owner.to_string(),
    //             admin: Some(owner.to_string()),
    //             config: VaultParametersConfig {},
    //             rebalancer: VaultRebalancerInstantiationMsg::Admin {}
    //         },
    //         &[],
    //         "my contract",
    //         None
    //     ).unwrap();
    // }

    #[test]
    fn serialization() {
        let msg = InstantiateMsg {
            pool: "pool".to_string(),
            admin: Some("owner".to_string()),
            config: VaultParametersConfig {},
            rebalancer: VaultRebalancerInstantiationMsg::Admin {}
        };

        let serialized: Binary = to_json_binary(&msg).unwrap();
        println!("Serialized: {:?}", serialized);
    }

}
