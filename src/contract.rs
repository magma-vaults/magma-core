use cosmwasm_std::{entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, Uint128};
use cw20_base::contract::execute_mint;
use cw20_base::state::{TokenInfo, TOKEN_INFO};
use std::cmp;

use crate::msg::{CalcSharesAndUsableAmountsResponse, PositionBalancesWithFeesResponse, QueryMsg, VaultBalancesResponse};
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

    // TODO Better Error types!!!!
    let vault_info = VaultInfo::new(msg.vault_info.clone(), deps.as_ref())?;
    // Invaraint: `VaultInfo` serialization should never fail.
    VAULT_INFO.save(deps.storage, &vault_info).unwrap();

    let vault_parameters = VaultParameters::new(msg.vault_parameters)?;
    // Invariant: `VaultParameters` serialization should never fail.
    VAULT_PARAMETERS.save(deps.storage, &vault_parameters).unwrap();

    let vault_state = VaultState::new();
    // Invariant: `VaultState` serialization should never fail.
    VAULT_STATE.save(deps.storage, &vault_state).unwrap();

    let token_info = TokenInfo {
        name: msg.vault_info.vault_name,
        symbol: msg.vault_info.vault_symbol,
        decimals: 18,
        total_supply: Uint128::zero(),
        mint: None
    };
    // Invariant: `TokenInfo` serialization should never fail.
    TOKEN_INFO.save(deps.storage, &token_info).unwrap();

    Ok(Response::new())
}


#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    use QueryMsg::*;
    match msg {
        VaultBalances {} => 
            Ok(to_json_binary(&query::vault_balances(deps, &env))?),
        PositionBalancesWithFees { position_type } => 
            Ok(to_json_binary(&query::position_balances_with_fees(position_type, deps))?),
        CalcSharesAndUsableAmounts { for_amount0, for_amount1 } => {
            Ok(to_json_binary(&query::calc_shares_and_usable_amounts(for_amount0, for_amount1, deps, &env))?)
        }
    }
}

mod query {
    use std::str::FromStr;

    use osmosis_std::types::{
        cosmos::bank::v1beta1::BankQuerier,
        osmosis::concentratedliquidity::v1beta1::PositionByIdRequest
    };

    use cosmwasm_std::Uint256;

    use super::*;
    use crate::msg::PositionType;

    pub fn vault_balances(deps: Deps, env: &Env) -> VaultBalancesResponse {
        use PositionType::*;
        let full_range_balances = position_balances_with_fees(FullRange, deps); 
        let base_balances = position_balances_with_fees(Base, deps);
        let limit_balances = position_balances_with_fees(Limit, deps);

        // Invariant: `VAULT_INFO` will always be present after instantiation.
        let (denom0, denom1) = VAULT_INFO.load(deps.storage).unwrap()
            .pool_id.denoms(&deps.querier);

        let balances = BankQuerier::new(&deps.querier);
        let contract_addr = env.contract.address.to_string();

        // Invariant: Wont return `None` becuase we verify the pool and
        //            denoms are proper during instantiation.
        let contract_balance0 = balances
            .balance(contract_addr.clone(), denom0.clone())
            .ok().unwrap().balance.unwrap().amount;

        let contract_balance1 = balances
            .balance(contract_addr, denom1.clone())
            .ok().unwrap().balance.unwrap().amount;

        // Invariant: The conversion wont fail, because we got the
        //            contract balances directly from `BankQuerier.
        //            The additions wont overflow, because for that
        //            the token supply would have to be above
        //            `Uint128::MAX`, but thats not possible.
        let bal0 = Uint128::from_str(&contract_balance0).unwrap()
            .checked_add(full_range_balances.bal0).unwrap()
            .checked_add(base_balances.bal0).unwrap()
            .checked_add(limit_balances.bal0).unwrap();

        let bal1 = Uint128::from_str(&contract_balance1).unwrap()
            .checked_add(full_range_balances.bal1).unwrap()
            .checked_add(base_balances.bal1).unwrap()
            .checked_add(limit_balances.bal1).unwrap();

        VaultBalancesResponse { bal0, bal1 }
    }

    pub fn position_balances_with_fees(position_type: PositionType, deps: Deps) -> PositionBalancesWithFeesResponse {
        // Invariant: `VAULT_INFO` should always be present after instantiation.
        let (denom0, denom1) = VAULT_INFO.load(deps.storage)
            .unwrap().pool_id.denoms(&deps.querier);

        // Invariant: `VAULT_STATE` should always be present after instantiation.
        let VaultState { 
            full_range_position_id,
            base_position_id,
            limit_position_id 
        } = VAULT_STATE.load(deps.storage).unwrap();

        use PositionType::*;
        let id = match position_type {
            FullRange => full_range_position_id,
            Base => base_position_id,
            Limit => limit_position_id
        };
        
        if id.is_none() { return PositionBalancesWithFeesResponse {
            bal0: Uint128::zero(),
            bal1: Uint128::zero()
        }}
        let id = id.unwrap();

        // Invariant: We verified `id` is a valid position id the moment
        //            we put it in the state, so the query wont fail.
        let pos = PositionByIdRequest { position_id: id }
            .query(&deps.querier).unwrap().position.unwrap();

        // Invariant: If position is valid, both assets will be always present.
        let asset0 = pos.asset0.unwrap();
        let asset1 = pos.asset1.unwrap();
        let rewards = pos.claimable_spread_rewards;

        assert!(denom0 == asset0.denom && denom1 == asset1.denom);
        
        // Invariant: If `rewards` is present, we know its a `Vec` of valid
        //            amounts, so the conversion will never fail.
        let rewards0 = rewards.iter()
            .find(|x| x.denom == denom0)
            .map(|x| Uint128::from_str(&x.amount))
            .unwrap_or(Ok(Uint128::zero())).unwrap();

        let rewards1 = rewards.iter()
            .find(|x| x.denom == denom1)
            .map(|x| Uint128::from_str(&x.amount))
            .unwrap_or(Ok(Uint128::zero())).unwrap();

        // Invariant: Will never panic, because if the position has amounts
        //            `amount0` and `amount1`, we know theyre valid `Uint128`s.
        //            Neither will the addition overflow, because balances
        //            could only overflow if the tokens they refer had
        //            supplies above `Uint128::MAX`, but thats not possible.
        let bal0 = Uint128::from_str(&asset0.amount).unwrap().checked_add(rewards0).unwrap();
        let bal1 = Uint128::from_str(&asset1.amount).unwrap().checked_add(rewards1).unwrap();

        PositionBalancesWithFeesResponse { bal0, bal1 }
    }

    pub fn calc_shares_and_usable_amounts(
        input_amount0: Uint128,
        input_amount1: Uint128,
        deps: Deps,
        env: &Env
    ) -> CalcSharesAndUsableAmountsResponse {

        let VaultBalancesResponse { 
            bal0: total0,
            bal1: total1
        } = query::vault_balances(deps, env);

        // Invariant: `TOKEN_INFO` always present after instantiation.
        let total_supply = TOKEN_INFO.load(deps.storage).unwrap().total_supply;

        if total_supply.is_zero() {
            assert!(total0.is_zero() && total1.is_zero());

            CalcSharesAndUsableAmountsResponse {
                shares: (cmp::max(input_amount0, input_amount1)),
                usable_amount0: input_amount0,
                usable_amount1: input_amount1
            }
        } else if total0.is_zero() {
            // Invariant: If there are shares and there are no tokens
            //            denom0 in the vault, then the shares must
            //            be for the token denom1.
            assert!(!total1.is_zero());

            // Invariant: The multiplication wont overflow becuase we
            //            lifted the amount to `Uint256`. The division
            //            wont fail becuase we just ensured `total1`
            //            is not zero. The downgrade back to `Uint128`
            //            wont fail because we divided proportionally
            //            by `total1`. The same reasoning applies to
            //            the rest of branches.
            let shares = Uint256::from(input_amount1)
                .checked_mul(total_supply.into()).unwrap()
                .checked_div(total1.into()).unwrap()
                .try_into().unwrap();

            CalcSharesAndUsableAmountsResponse {
                shares,
                usable_amount0: Uint128::zero(),
                usable_amount1: input_amount1
            }
        } else if total1.is_zero() {
            // Invariant: If there are shares and there are no tokens
            //            denom1 in the vault, then the shares must
            //            be for the token denom0.
            assert!(!total0.is_zero());

            let shares = Uint256::from(input_amount0)
                .checked_mul(total_supply.into()).unwrap()
                .checked_div(total0.into()).unwrap()
                .try_into().unwrap();

            CalcSharesAndUsableAmountsResponse {
                shares,
                usable_amount0: input_amount0,
                usable_amount1: Uint128::zero()
            }
        } else {
            let input_amount0: Uint256 = input_amount0.into();
            let input_amount1: Uint256 = input_amount1.into();
            let total0: Uint256 = total0.into();
            let total1: Uint256 = total1.into();

            let cross = cmp::min(
                input_amount0.checked_mul(total1.into()).unwrap(),
                input_amount1.checked_mul(total0.into()).unwrap()
            );
            
            if cross.is_zero() {
                return CalcSharesAndUsableAmountsResponse {
                    shares: Uint128::zero(),
                    usable_amount0: Uint128::zero(),
                    usable_amount1: Uint128::zero()
                }
            } 

            let usable_amount0 = cross
                .checked_sub(Uint256::one()).unwrap()
                .checked_div(total1).unwrap()
                .checked_add(Uint256::one()).unwrap()
                .try_into().unwrap();

            let usable_amount1 = cross
                .checked_sub(Uint256::one()).unwrap()
                .checked_div(total0).unwrap()
                .checked_add(Uint256::one()).unwrap()
                .try_into().unwrap();

            let shares = cross
                .checked_mul(total_supply.into()).unwrap()
                .checked_div(total0).unwrap()
                .checked_div(total1).unwrap()
                .try_into().unwrap();

            CalcSharesAndUsableAmountsResponse {
                shares,
                usable_amount0,
                usable_amount1 
            }
        }
    }
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
    use osmosis_std::types::{
        cosmos::bank::v1beta1::BankQuerier,
        osmosis::concentratedliquidity::v1beta1::MsgCreatePosition
    };
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

        // TODO: Handle better decimal conversion errors.
        let amount0 = Uint128::from_str(&amount0)?;
        let amount1 = Uint128::from_str(&amount1)?;
        let amount0_min = Uint128::from_str(&amount0_min)?;
        let amount1_min = Uint128::from_str(&amount1_min)?;

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

        let CalcSharesAndUsableAmountsResponse {
            shares, 
            usable_amount0: amount_used0,
            usable_amount1: amount_used1
        } = query::calc_shares_and_usable_amounts(amount0, amount1, deps.as_ref(), &env);

        if shares.is_zero() {
            return Err(ContractError::InvalidDeposit {})
        }

        // TODO Whats `MINIMUM_LIQUIDITY`? Probably some hack to prevent weird divisions by 0.

        // TODO: Document this invariant. ITS NOT a requirement, even if it looks like.
        assert!(amount_used0 <= amount0 && amount_used1 <= amount1);

        let refunded_amounts = vec![
            Coin {denom: denom0, amount: amount0 - amount_used0},
            Coin {denom: denom1, amount: amount1 - amount_used1}
        ];

        if amount0 < amount0_min || amount1 < amount1_min {
            return Err(ContractError::InvalidDeposit {})
        }

        // Invariant: We already verified the holder, same with the shares.
        execute_mint(deps, env, info.clone(), new_holder.to_string(), shares)
            .unwrap();

        Ok(Response::new().add_message(BankMsg::Send { 
            to_address: info.sender.to_string(),
            amount: refunded_amounts 
        }))
    }

    // TODO I havent even really started cleaning the hard pad did I?
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
            base_factor, limit_factor, full_range_weight
        } = vault_parameters;

        // We take 1% slippage.
        let slippage = Weight::new(&"0.99".into()).unwrap();

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

        // TODO Fix business logic.
        if !full_range_weight.is_zero() {
            // TODO What if we can only put a limit order? Then the math breaks!

            let full_range_tokens = vec![
                Coin { denom: pool.token0.clone(), amount: raw(&full_range_balance0) },
                Coin { denom: pool.token1.clone(), amount: raw(&full_range_balance1) }
            ];

            let _full_range_position = MsgCreatePosition {
                pool_id: pool.id,
                sender: contract_addr.clone(),
                lower_tick: pool_id.min_valid_tick(&deps.querier),
                upper_tick:  pool_id.max_valid_tick(&deps.querier),
                tokens_provided: full_range_tokens,
                token_min_amount0: raw(&slippage.mul_dec(&full_range_balance0)),
                token_min_amount1: raw(&slippage.mul_dec(&full_range_balance1))
            };
        }

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

        if !limit_factor.is_one() {
            // TODO
        }

        unimplemented!()
    }
}

