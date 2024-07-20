use cosmwasm_std::{entry_point, to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult};
use cw20_base::contract::{execute_mint, query_token_info};
use osmosis_std::types::osmosis::{self, concentratedliquidity::v1beta1::Pool, poolmanager::v1beta1::PoolmanagerQuerier};

use crate::{
    error::ContractError, msg::{ExecuteMsg, InstantiateMsg, VaultInfoInstantaiteMsg, VaultParametersInstantiateMsg}, state::{
        PoolId, VaultRebalancer, Weight, VAULT_STATE
    }
};

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg
) -> Result<Response, ContractError> {

    let VaultInfoInstantaiteMsg { pool_id, denom0, denom1, .. } = msg.vault_info;
    let qq = deps.querier;
    let q = PoolmanagerQuerier::new(&deps.querier);

    let pool_id = PoolId::new(msg.vault_info.pool_id, &deps.querier)
        .ok_or(ContractError::InvalidConfig {})?;

    let pool = pool_id.to_pool(&deps.querier);

    // NOTE TODO FIXME We dont need to pass those as args anymore!
    // Same for tick spacing!
    let denom0 = msg.vault_info.denom0;
    let denom1 = msg.vault_info.denom1;
    if let Some(serialized_pool) = q.pool(pool_id)?.pool {
        // The pool could only not be deserialized if `pool_id` does not refer
        // to a valid concentrated liquidity pool.
        let p = Pool::try_from(serialized_pool)
            .map_err(|_| ContractError::InvalidPoolId(pool_id))?;
        
        assert!(p.id == pool_id);
    }

    let state = msg
        .validate(deps.as_ref())
        .ok_or(ContractError::InvalidConfig {})?;

    if let None = state.vault_management_config.admin {
        match state.vault_management_config.rebalancer {
            VaultRebalancer::Anyone {} => Ok(()),
            _ => Err(ContractError::InvalidConfig {}),
        }?
    }

    // Creating the 3 positions?
    let pos1 = MsgCreatePosition {
        pool_id: msg.vault_info.pool_id
        // lower_tick: msg.
    };

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
