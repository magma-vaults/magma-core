use cosmwasm_std::{entry_point, to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult};
use cw20_base::contract::{execute_mint, query_token_info};
use std::cmp;

use crate::{
    error::ContractError, msg::{ExecuteMsg, InstantiateMsg}, state::{
        VaultInfo, VaultParameters, VaultState, VAULT_INFO, VAULT_PARAMETERS, VAULT_STATE
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
    // Invaraint: `VaultInfo` serialization should never fail.
    VAULT_INFO.save(deps.storage, &vault_info).unwrap();

    let vault_parameters = VaultParameters::new(msg.vault_parameters)?;
    // Invariant: `VaultParameters` serialization should never fail.
    VAULT_PARAMETERS.save(deps.storage, &vault_parameters).unwrap();

    let vault_state = VaultState::new();
    // Invariant: `VaultState` serialization should never fail.
    VAULT_STATE.save(deps.storage, &vault_state).unwrap();

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

    use core::num;
    use std::str::FromStr;
    use cosmwasm_std::{Decimal, Uint128};
    use osmosis_std::types::{cosmos::bank::v1beta1::BankQuerier, osmosis::concentratedliquidity::v1beta1::{ConcentratedliquidityQuerier, MsgCreatePosition}};
    use crate::{msg::DepositMsg, state::{price_function_inv, raw, Weight}};
    use super::*;

    // TODO More clarifying errors. TODO Events to query positions (deposits).
    pub fn deposit(
        DepositMsg { amount0, amount1, amount0_min, amount1_min, to }: DepositMsg,
        deps: DepsMut,
        env: Env,
        info: MessageInfo
    ) -> Result<Response, ContractError> {
        use cosmwasm_std::{BankMsg, Coin, Uint128};
        
        // Invariant: `VAULT_INFO` should always be present after instantiation.
        let vault_info = VAULT_INFO.load(deps.storage).unwrap();

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

        // TODO Whats `MINIMUM_LIQUIDITY`? Probably some hack to prevent
        // weird divisions by 0.
        let (new_shares, amount0_used, amount1_used) = {
            let total_supply = query_token_info(deps.as_ref())?.total_supply;

            // TODO Calc position amounts. Absolute! What if someone else 
            // deposists to that position outside of the vault?
            let (total0, total1) = {
                // Invariant: `VAULT_STATE` should always be present after instantiation.
                let VaultState {
                    full_range_position_id,
                    base_position_id,
                    limit_position_id
                } = VAULT_STATE.load(deps.storage).unwrap();

                let position_querier = ConcentratedliquidityQuerier::new(&deps.querier);
                let mut total0 = Decimal::zero();
                let mut total1 = Decimal::zero();

                if let Some(full_range_id) = full_range_position_id {
                    // Invariant: We know that `full_range_id`, if present,
                    //            refers to a valid CL position.
                    let pos = position_querier.position_by_id(full_range_id)
                        .unwrap().position.unwrap();

                    // TODO: Prove computation security. 
                    //       Why wouldnt asset0 and asset1 be present?
                    let full_range_amount0 = Decimal::from_str(
                        &pos.asset0.unwrap().amount
                    ).unwrap();

                    let full_range_amount1 = Decimal::from_str(
                        &pos.asset1.unwrap().amount
                    ).unwrap();

                    // TODO: Prove computation security.
                    total0 = total0.checked_add(full_range_amount0).unwrap();
                    total1 = total1.checked_add(full_range_amount1).unwrap();
                }
                
                if let Some(base_id) = base_position_id {
                    // Invariant: We know that `base_id`, if present,
                    //            refers to a valid CL position.
                    let pos = position_querier.position_by_id(base_id)
                        .unwrap().position.unwrap();

                    // TODO: Prove computation security. 
                    //       Why wouldnt asset0 and asset1 be present?
                    let base_pos_amount0 = Decimal::from_str(
                        &pos.asset0.unwrap().amount
                    ).unwrap();

                    let base_pos_amount1 = Decimal::from_str(
                        &pos.asset1.unwrap().amount
                    ).unwrap();

                    // TODO: Prove computation security.
                    total0 = total0.checked_add(base_pos_amount0).unwrap();
                    total1 = total1.checked_add(base_pos_amount1).unwrap();
                }

                if let Some(limit_id) = limit_position_id {
                    // Invariant: We know that `limit_id`, if present,
                    //            refers to a valid CL position.
                    let pos = position_querier.position_by_id(limit_id)
                        .unwrap().position.unwrap();

                    // TODO: Prove computation security. 
                    //       Why wouldnt asset0 and asset1 be present?
                    let limit_pos_amount0 = Decimal::from_str(
                        &pos.asset0.unwrap().amount
                    ).unwrap();

                    let limit_pos_amount1 = Decimal::from_str(
                        &pos.asset1.unwrap().amount
                    ).unwrap();

                    // TODO: Prove computation security.
                    total0 = total0.checked_add(limit_pos_amount0).unwrap();
                    total1 = total1.checked_add(limit_pos_amount1).unwrap();
                }
                
                if true {
                    // TODO: Compute fee amounts.
                    total0 += Decimal::zero();
                    total1 += Decimal::zero();
                }

                (total0.atomics(), total1.atomics())
            };

            // TODO Formalize CharmFi shares calculation model.
            if total_supply.is_zero() {
                // Invariant: If there are no shares, then there shouldnt be
                //            any vault tokens for that shares.
                assert!(total0.is_zero() && total1.is_zero());

                (cmp::max(amount0, amount1), amount0, amount1)
            } else if total0.is_zero() {
                // Invariant: If there are shares and there are no tokens
                //            denom0 in the vault, then the shares must
                //            be for the token denom1.
                assert!(!total1.is_zero());

                // TODO Prove computation security.
                let shares = amount1
                    .checked_mul(total_supply)
                    .unwrap()
                    .checked_div(total1)
                    .unwrap();

                (shares, Uint128::zero(), amount1)
            } else if total1.is_zero() {
                // Invariant: If there are shares and there are no tokens
                //            denom1 in the vault, then the shares must
                //            be for the token denom0.
                assert!(!total0.is_zero());

                // TODO Prove computation security.
                let shares = amount0
                    .checked_mul(total_supply)
                    .unwrap()
                    .checked_div(total0)
                    .unwrap();

                (shares, amount0, Uint128::zero())
            } else {
                // TODO: Prove computation security.
                let cross = cmp::min(
                    amount0.checked_mul(total0).unwrap(),
                    amount1.checked_mul(total1).unwrap()
                );
                // TODO: Is this an invariant or a requirement?
                assert!(cross > Uint128::zero());

                let amount0_used = cross
                    .checked_sub(Uint128::one())
                    .unwrap() // Invariant: We already verified `cross > 0`.
                    .checked_div(total1)
                    .unwrap() // TODO: Prove computation security.
                    .checked_add(Uint128::one())
                    .unwrap(); // TODO: Prove computation security.

                let amount1_used = cross
                    .checked_sub(Uint128::one())
                    .unwrap() // Invariant: We already verified `cross > 0`.
                    .checked_div(total0)
                    .unwrap() // TODO: Prove computation security.
                    .checked_add(Uint128::one())
                    .unwrap(); // TODO: Prove computation security.

                // TODO: Prove computation security.
                let shares = cross
                    .checked_mul(total_supply)
                    .unwrap()
                    .checked_div(total0)
                    .unwrap()
                    .checked_div(total1)
                    .unwrap();

                (shares, amount0_used, amount1_used)
            }
        };

        // TODO: Document invariants.
        assert!(amount0_used <= amount0 && amount1_used <= amount1);
        assert!(!new_shares.is_zero());

        let refunded_amounts = vec![
            Coin {denom: denom0, amount: amount0 - amount0_used},
            Coin {denom: denom1, amount: amount1 - amount1_used}
        ];

        if amount0 < amount0_min.into() || amount1 < amount1_min.into() {
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

        let VaultParameters { 
            base_factor, limit_factor, full_range_weight, .. 
        } = vault_parameters;
        // We take 1% slippage.
        let slippage = Weight::new("0.99".to_string()).unwrap();

        // TODO Fix business logic.
        if !full_range_weight.is_zero() {
            // TODO What if we can only put a limit order? Then the math breaks!
            let (full_range_balance0, full_range_balance1) = {
                // TODO Document the math (see [[MagmaLiquidity]]).
                // FIXME All those unwraps could fail under extreme conditions.
                let sqrt_k = base_factor.0.sqrt();

                let numerator = full_range_weight
                    .mul_dec(&sqrt_k.sqrt())
                    .checked_mul(balanced_balance0)
                    .ok();

                let denominator = sqrt_k
                    .checked_sub(Decimal::one())
                    .unwrap() // Invariant: `k` min value is 1, `sqrt(1) - 1 == Decimal::zero()`
                    .checked_add(full_range_weight.0)
                    .unwrap(); // Invariant: `w` max value is 1, and we already subtracted 1.

                let x0 = numerator.and_then(|n| n.checked_div(denominator).ok()).unwrap();
                let y0 = x0.checked_mul(price).unwrap();
                (x0, y0)
            };

            let full_range_tokens = vec![
                Coin { denom: pool.token0.clone(), amount: raw(&full_range_balance0) },
                Coin { denom: pool.token1.clone(), amount: raw(&full_range_balance1) }
            ];

            let full_range_position = MsgCreatePosition {
                pool_id: pool.id,
                sender: contract_addr.clone(),
                lower_tick: pool_id.min_valid_tick(&deps.querier),
                upper_tick:  pool_id.max_valid_tick(&deps.querier),
                tokens_provided: full_range_tokens,
                token_min_amount0: raw(&slippage.mul_dec(&full_range_balance0)),
                token_min_amount1: raw(&slippage.mul_dec(&full_range_balance1))
            };
        }

        let base_factor = vault_parameters.base_factor;

        if !base_factor.is_one() {

            // TODO Prove that those unwraps will never fail.
            let base_range_balance0 = balanced_balance0
                .checked_sub(full_range_balance0)
                .unwrap();
            let base_range_balance1 = balanced_balance1
                .checked_sub(full_range_balance1)
                .unwrap();

            let base_range_tokens = vec![
                Coin { denom: pool.token0, amount: raw(&base_range_balance0) },
                Coin { denom: pool.token1, amount: raw(&base_range_balance1) }
            ];


            let current_price = pool_id.price(&deps.querier);
            let lower_price = base_factor.0
                .checked_div(current_price)
                .unwrap_or(Decimal::MIN);

            let upper_price = base_factor.0
                .checked_mul(current_price)
                .unwrap_or(Decimal::MAX);

            let lower_tick = pool_id.closest_valid_tick(
                price_function_inv(&lower_price), &deps.querier
            );

            let upper_tick = pool_id.closest_valid_tick(
                price_function_inv(&upper_price), &deps.querier
            );
                
            let _base_range_position = MsgCreatePosition {
                pool_id: pool.id,  
                sender: contract_addr,
                lower_tick,
                upper_tick,
                tokens_provided: base_range_tokens,
                token_min_amount0: raw(&slippage.mul_dec(&base_range_balance0)),
                token_min_amount1: raw(&slippage.mul_dec(&base_range_balance1))
            };
        }



        unimplemented!()  
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use cosmwasm_std::Decimal256;

    #[test]
    fn decimal_type_test() {
        let target_price = "1.300904508667263332308471677698306399".to_string();

        Decimal256::from_atomics(atomics, decimal_places)
        let d = Decimal256::from_str(&target_price)
            .unwrap() // Invariant: Pools always hold valid prices as decimals.
            .checked_pow(2)
            .unwrap(); // Invariant: `sqrt(Decimal::MAX)^2 == Decimal::MAX`
        
        println!("{}", d);
    }
}
