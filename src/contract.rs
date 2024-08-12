use cosmwasm_std::{entry_point, to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult};
use cw20_base::contract::{execute_mint, query_token_info};
use std::cmp;

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
        Rebalance {} => exec::rebalance(deps.as_ref(), env),
    }
}

mod exec {

    use std::str::FromStr;
    use cosmwasm_std::{BankMsg, Coin, Uint128};
    use osmosis_std::types::cosmos::bank::v1beta1::BankQuerier;
    use crate::msg::DepositMsg;
    use super::*;

    // TODO More clarifying errors. TODO Events to query positions (deposits).
    pub fn deposit(
        DepositMsg { amount0, amount1, amount0_min, amount1_min, to }: DepositMsg,
        deps: DepsMut,
        env: Env,
        info: MessageInfo
    ) -> Result<Response, ContractError> {
        
        let vault_info = VAULT_INFO.load(deps.storage)?;
        let denom0 = vault_info.demon0(&deps.querier);
        let denom1 = vault_info.demon1(&deps.querier);
        let amount0 = Uint128::from(amount0);
        let amount1 = Uint128::from(amount1);

        let expected_amounts = vec![
            Coin {denom: denom0.clone(), amount: amount0},
            Coin {denom: denom1.clone(), amount: amount1}
        ];

        if expected_amounts != info.funds {
            return Err(ContractError::InvalidDeposit {})
        }

        if amount0.is_zero() && amount1.is_zero() {
            return Err(ContractError::InvalidDeposit {})
        }

        let new_holder = deps.api.addr_validate(&to)?;

        if new_holder == env.contract.address {
            return Err(ContractError::InvalidDeposit {})
        }

        // TODO Whats `MINIMUM_LIQUIDITY`?
        let (new_shares, amount0_used, amount1_used) = {
            let total_supply = query_token_info(deps.as_ref())?.total_supply;

            // TODO Calc position amounts. Absolute! What if someone else 
            // deposists to that position outside of the vault?
            let total0: Uint128 = Uint128::zero();
            let total1: Uint128 = Uint128::zero();

            if total_supply.is_zero() {
                (cmp::max(amount0, amount1), amount0, amount1)
            } else if total0.is_zero() {
                // TODO Why? Research first rebalance impact on totals.
                ((amount0 * total_supply)/total0, Uint128::zero(), amount1)
            } else if total1.is_zero() {
                // TODO Why? Research first rebalance impact on totals.
                ((amount1 * total_supply)/total1, amount0, Uint128::zero())
            } else {
                // TODO Why? Research first rebalance impact on totals.
                let cross = cmp::min(amount0 * total0, amount1 * total1);
                assert!(cross > Uint128::zero());

                let amount0_used = (cross - Uint128::one())/total1 + Uint128::one();
                let amount1_used = (cross - Uint128::one())/total0 + Uint128::one();
                ((cross * total_supply)/(total0 * total1), amount0_used, amount1_used)
            }
        };

        assert!(amount0_used <= amount0 && amount1_used <= amount1);

        let refunded_amounts = vec![
            Coin {denom: denom0, amount: amount0 - amount0_used},
            Coin {denom: denom1, amount: amount1 - amount1_used}
        ];

        if amount0 < amount0_min.into() || amount1 < amount1_min.into() {
            return Err(ContractError::InvalidDeposit {})
        }

        if new_shares.is_zero() {
            return Err(ContractError::InvalidDeposit {})
        }

        execute_mint(deps, env, info.clone(), new_holder.to_string(), new_shares.into())?;

        Ok(Response::new().add_message(BankMsg::Send { 
            to_address: info.sender.to_string(), amount: refunded_amounts 
        }))
    }

    pub fn rebalance(deps: Deps, env: Env) -> Result<Response, ContractError> {
        // TODO Can rebalance? Check `VaultRebalancer` and other params,
        // like `minTickMove` or `period`.

        // TODO Withdraw current liquidities.

        let vault_info = VAULT_INFO.load(deps.storage)?;
        let _vault_parameters = VAULT_PARAMETERS.load(deps.storage)?;
        let pool_id = &vault_info.pool_id;
        let pool = pool_id.to_pool(&deps.querier);
        let contract_addr = env.contract.address.to_string();

        let balances = BankQuerier::new(&deps.querier);
        let coin0_res = balances.balance(contract_addr.clone(), pool.token0.clone())?;
        let coin1_res = balances.balance(contract_addr.clone(), pool.token1.clone())?;

        let _balance0 = if let Some(coin0) = coin0_res.balance {
            assert!(coin0.denom == pool.token0);
            Uint128::from_str(&coin0.amount)?
        } else { Uint128::zero() };

        let _balance1 = if let Some(coin1) = coin1_res.balance {
            assert!(coin1.denom == pool.token1);
            Uint128::from_str(&coin1.amount)?
        } else { Uint128::zero() };


        // Full range position. TODO HOW... Calc liquidities... it should be possible.

        // let full_range_pos = MsgCreatePosition {
        //     pool_id: pool.id,
        //     sender: contract_addr,
        //     lower_tick: pool_id.min_valid_tick(&deps.querier).0,
        //     upper_tick: pool_id.max_valid_tick(&deps.querier).0
        // };
        unimplemented!()  
    }

}
