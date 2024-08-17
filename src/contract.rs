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
    use osmosis_std::types::{cosmos::bank::v1beta1::BankQuerier, osmosis::concentratedliquidity::v1beta1::MsgCreatePosition};
    use crate::msg::DepositMsg;
    use super::*;

    // TODO More clarifying errors. TODO Events to query positions (deposits).
    pub fn deposit(
        DepositMsg { amount0, amount1, amount0_min, amount1_min, to }: DepositMsg,
        deps: DepsMut,
        env: Env,
        info: MessageInfo
    ) -> Result<Response, ContractError> {
        use cosmwasm_std::{BankMsg, Coin, Uint128};
        
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
        use cosmwasm_std::Decimal;
        use osmosis_std::types::cosmos::base::v1beta1::Coin;
        // TODO Can rebalance? Check `VaultRebalancer` and other params,
        // like `minTickMove` or `period`.

        // TODO Withdraw current liquidities.

        let vault_info = VAULT_INFO.load(deps.storage)?;
        let vault_parameters = VAULT_PARAMETERS.load(deps.storage)?;
        let pool_id = &vault_info.pool_id;
        let pool = pool_id.to_pool(&deps.querier);
        let contract_addr = env.contract.address.to_string();

        let balances = BankQuerier::new(&deps.querier);
        let coin0_res = balances.balance(contract_addr.clone(), pool.token0.clone())?;
        let coin1_res = balances.balance(contract_addr.clone(), pool.token1.clone())?;

        let balance0 = if let Some(coin0) = coin0_res.balance {
            assert!(coin0.denom == pool.token0);
            // Invariant: We know `coin0_res` holds a valid Decimal as String.
            Decimal::from_str(&coin0.amount).unwrap()
        } else { Decimal::zero() };

        let balance1 = if let Some(coin1) = coin1_res.balance {
            assert!(coin1.denom == pool.token1);
            // Invariant: We know `coin0_res` holds a valid Decimal as String.
            Decimal::from_str(&coin1.amount)?
        } else { Decimal::zero() };

        let price = pool_id.price(&deps.querier);

        let (balanced_balance0, balanced_balance1) = {
            // FIXME Those could overflow under extreme conditions, both the
            // division and the multiplication.
            let balanced0 = balance0.checked_div(price).unwrap();
            let balanced1 = balance1.checked_mul(price).unwrap();

            if balanced0 > balance0 { (balance0, balanced1) } 
            else { (balanced0, balance1) }
        };

        assert!(balance0 >= balanced_balance0 && balance1 >= balanced_balance1);

        // TODO What if we can only put a limit order? Then the math breaks!
        let (full_range_balance0, full_range_balance1) = {
            // TODO Document the math (see [[MagmaLiquidity]]).
            // FIXME All those unwraps could fail under extreme conditions.
            let sqrt_k = vault_parameters.base_factor.0.sqrt();
            let w = vault_parameters.full_range_weight.0;
            let x = balanced_balance0;

            let numerator = sqrt_k
                .sqrt()
                .checked_mul(w)
                .and_then(|n| n.checked_mul(x))
                .ok();

            let denominator = sqrt_k
                .checked_sub(Decimal::one())
                .unwrap() // Invariant: `k` min value is 1, `sqrt(1) - 1 == Decimal::zero()`
                .checked_add(w)
                .unwrap(); // Invariant: `w` max value is 1, and we already subtracted 1.

            let x0 = numerator.and_then(|n| n.checked_div(denominator).ok()).unwrap();
            let y0 = x0.checked_mul(price).unwrap();
            (x0, y0)
        };

        let full_range_tokens = vec![
            Coin { denom: pool.token0, amount: full_range_balance0.atomics().into() },
            Coin { denom: pool.token1, amount: full_range_balance1.atomics().into() }
        ];

        let full_range_position = MsgCreatePosition {
            pool_id: pool.id,
            sender: contract_addr,
            lower_tick: -100, // TODO
            upper_tick:  100, // TODO
            tokens_provided: full_range_tokens,
            token_min_amount0: full_range_balance0.atomics().into(),
            token_min_amount1: full_range_balance1.atomics().into()
        };

        // TODO Prove that those unwraps will never fail.
        let base_range_balance0 = balanced_balance0
            .checked_sub(full_range_balance0)
            .unwrap();
        let base_range_balance1 = balanced_balance1
            .checked_sub(full_range_balance1)
            .unwrap();

        let base_range_tokens = vec![
            Coin { denom: pool.token0, amount: base_range_balance0.atomics().into() },
            Coin { denom: pool.token1, amount: base_range_balance1.atomics().into() }
        ]

        let base_range_position = MsgCreatePosition {
            pool_id: pool.id,  
            sender: contract_addr,
            lower_tick: 
        }


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
