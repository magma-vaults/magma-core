use cosmwasm_std::{entry_point, to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult};
use cw20_base::contract::{execute_mint, query_token_info};

use crate::{
    error::ContractError, msg::{ExecuteMsg, InstantiateMsg}, state::{
        VaultInfo, VaultParameters, VAULT_INFO, VAULT_PARAMETERS
    }
};

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg
) -> Result<Response, ContractError> {

    let vault_info = VaultInfo::new(msg.vault_info, deps.as_ref())?;
    VAULT_INFO.save(deps.storage, &vault_info)?;

    let vault_parameters = VaultParameters::new(msg.vault_parameters, vault_info, &deps.querier)?;
    VAULT_PARAMETERS.save(deps.storage, &vault_parameters)?;

    // let pool = vault_info.pool_id.to_pool(&deps.querier);

    // let base_pos = MsgCreatePosition {
    //     pool_id: pool.id,
    //     sender: info.sender.to_string(),
    //     lower_tick: pool.current_tick - vault_parameters.base_threshold.0.into(),
    //     upper_tick: pool.current_tick + vault_parameters.base_threshold.0.into(),
    //     tokens_provided: vec![],
    // };


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
    info: MessageInfo,
    msg: ExecuteMsg
) -> Result<Response, ContractError> {
    use ExecuteMsg::*;
    match msg {
        Deposit(deposit_msg) => exec::deposit(deposit_msg, deps, env, info),
    }
}

mod exec {


    use cosmwasm_std::{BankMsg, Coin};
    use cw20::TokenInfoResponse;
    use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::{ConcentratedliquidityQuerier, MsgCreatePositionResponse, PositionByIdRequest, PositionByIdResponse};

    use crate::{msg::DepositMsg, state::VaultInfo};

    use super::*;



    // TODO More clarifying errors. TODO Events to query positions (deposits).
    pub fn deposit(
        DepositMsg { amount0, amount1, amount0_min, amount1_min, to }: DepositMsg,
        deps: DepsMut,
        env: Env,
        info: MessageInfo
    ) -> Result<Response, ContractError> {
        
        let VaultInfo { denom0, denom1, .. } = VAULT_STATE
            .load(deps.storage)?
            .vault_info;

        let expected_amounts = vec![
            Coin {denom: denom0.clone(), amount: amount0.into()},
            Coin {denom: denom1.clone(), amount: amount1.into()}
        ];

        if expected_amounts != info.funds {
            return Err(ContractError::InvalidDeposit {})
        }

        if amount0 == 0 && amount1 == 0 {
            return Err(ContractError::InvalidDeposit {})
        }

        let new_holder = deps.api.addr_validate(&to)?;

        if new_holder == env.contract.address {
            return Err(ContractError::InvalidDeposit {})
        }

        let (new_shares, amount0_used, amount1_used) = {
            let TokenInfoResponse { 
                total_supply, .. 
            } = query_token_info(deps.as_ref())?;

            let shares: u128 = 3;
            (shares, 2, 3)
        };

        assert!(amount0_used <= amount0 && amount1_used <= amount1);

        let refunded_amounts = vec![
            Coin {denom: denom0, amount: (amount0 - amount0_used).into()},
            Coin {denom: denom1, amount: (amount1 - amount1_used).into()}
        ];

        if amount0 < amount0_min || amount1 < amount1_min {
            return Err(ContractError::InvalidDeposit {})
        }

        if new_shares == 0 {
            return Err(ContractError::InvalidDeposit {})
        }

        execute_mint(deps, env, info.clone(), new_holder.to_string(), new_shares.into())?;

        Ok(Response::new().add_message(BankMsg::Send { 
            to_address: info.sender.to_string(), amount: refunded_amounts 
        }))
    }

    pub fn rebalance() -> Result<Response, ContractError> {
        unimplemented!()  
    }

    pub fn test_swap(env: Env, deps: Deps) -> Result<Response, ContractError> {
        let querier = ConcentratedliquidityQuerier::new(&deps.querier);
        // querier.user_positions(address, pool_id, pagination)
        unimplemented!();
        /*
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
        */
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
