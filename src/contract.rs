use cosmwasm_std::{entry_point, to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdResult, Uint128};
use cw20_base::contract::{execute_mint, query_token_info};
use cw20_base::state::{TokenInfo, TOKEN_INFO};
use std::cmp;

use crate::msg::{PositionBalancesWithFeesResponse, QueryMsg, VaultBalancesResponse};
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
        VaultBalances {} => {
            let res = query::vault_balances(deps, env);
            // TODO Test if Coin serializes properly.
            Ok(to_json_binary(&VaultBalancesResponse { res })?)
        }, 
        PositionBalancesWithFees { position_type } =>  {
            let res = query::position_balances_with_fees(position_type, deps);
            // TODO Test if Coin serializes properly.
            Ok(to_json_binary(&PositionBalancesWithFeesResponse { res })?)
        }
    }
}

mod query {
    use std::str::FromStr;

    use osmosis_std::types::{cosmos::{bank::v1beta1::BankQuerier, base::v1beta1::Coin}, osmosis::concentratedliquidity::v1beta1::{FullPositionBreakdown, PositionByIdRequest}};

    use super::*;
    use crate::msg::{CoinsPair, PositionBalancesWithFeesResponse, PositionType, VaultBalancesResponse};

    pub fn vault_balances(deps: Deps, env: Env) -> CoinsPair {
        use PositionType::*;
        let full_range_balances = position_balances_with_fees(FullRange, deps); 
        let base_balances = position_balances_with_fees(Base, deps);
        let limit_balances = position_balances_with_fees(Limit, deps);

        let contract_balances = (|| {
            // Invariant: Wont return `None` because `VAULT_INFO` should always
            //            be present after instantiation.
            let (denom0, denom1) = VAULT_INFO.load(deps.storage).ok()?
                .pool_id.denoms(&deps.querier);

            let balances = BankQuerier::new(&deps.querier);
            let contract_addr = env.contract.address.to_string();

            // Invariant: Wont return `None` because we get the denoms directly
            //            from an already verified during instantiation pool.
            let contract_balance0 = balances
                .balance(contract_addr.clone(), denom0.clone())
                .ok()?.balance?.amount;

            let contract_balance1 = balances
                .balance(contract_addr, denom1.clone())
                .ok()?.balance?.amount;

            // Invariant: Wont return `None` because we just queried both balances.
            Some(CoinsPair::new(
                denom0, Uint128::from_str(&contract_balance0).ok()?,
                denom1, Uint128::from_str(&contract_balance1).ok()?
            ))
        })().unwrap(); // Invariant: Will never panic. Proof: See closure.

        // Invariant: None of those additions will return `None`. Balances 
        //            could only overflow if the tokens they refer had
        //            supplies above `Uint128::MAX`, but thats not possible.
        (|| full_range_balances
            .checked_add(base_balances)?
            .checked_add(limit_balances)?
            .checked_add(contract_balances)
        )().unwrap()
    }

    pub fn position_balances_with_fees(position_type: PositionType, deps: Deps) -> CoinsPair {
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
        
        if let None = id {
            return CoinsPair::new(denom0, Uint128::zero(), denom1, Uint128::zero())
        }
        let id = id.unwrap();

        // Invariant: We already verified `id` is a valid position id the moment
        //            we put it in the state, so the query wont fail.
        let pos = PositionByIdRequest { position_id: id }
            .query(&deps.querier).unwrap().position.unwrap();

        // Invariant: If `id` does not refer to a valid position id, we already
        //            just returned a `NonExistentPosition` error. Otherwise,
        //            any valid position will hold `asset0` and `asset1`,
        assert!(pos.asset0.is_some() && pos.asset1.is_some());
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

        // Invariant: Will never return panic, because if the position has
        //            amounts `amount0` and `amount1`, we know theyre valid
        //            `Uint128`s. Neither will the addition overflow, because
        //            balances could only overflow if the tokens they refer
        //            had supplies above `Uint128::MAX`, but thats not possible.
        let amount0 = Uint128::from_str(&asset0.amount).unwrap().checked_add(rewards0).unwrap();
        let amount1 = Uint128::from_str(&asset1.amount).unwrap().checked_add(rewards1).unwrap();

        CoinsPair::new(denom0, amount0, denom1, amount1)
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

    use core::num;
    use std::{io::empty, str::FromStr};
    use cosmwasm_std::{Decimal, StdError, Uint128, Uint256};
    use osmosis_std::types::{cosmos::bank::v1beta1::BankQuerier, osmosis::concentratedliquidity::v1beta1::{ConcentratedliquidityQuerier, FullPositionBreakdown, MsgCreatePosition, PositionByIdRequest}};
    use crate::{error::{DepositError, InvalidProportionError, PositionBalanceComputationError, SharesAndAmountsComputationError, TotalContractBalancesComputationError}, msg::DepositMsg, state::{price_function_inv, raw, Weight}};
    use super::*;

    /// Returns the amounts of all positions with fees.
    /// TODO: Also take into account holded balances that havent been used yet.
    fn vault_amounts() {
        unimplemented!()
    }


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
        let amount0 = Decimal::from_str(&amount0)?.atomics();
        let amount1 = Decimal::from_str(&amount1)?.atomics();
        let amount0_min = Decimal::from_str(&amount0_min)?.atomics();
        let amount1_min = Decimal::from_str(&amount1_min)?.atomics();


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

        let compute_shares_and_amounts = || -> Result<(Uint128, Uint128, Uint128), SharesAndAmountsComputationError> {
            use SharesAndAmountsComputationError::*;
            let total_supply = query_token_info(deps.as_ref())?.total_supply;

            let total_contract_balances_computation = || -> Result<(Uint128, Uint128), TotalContractBalancesComputationError> {
                // Invariant: `VAULT_STATE` should always be present after instantiation.
                let VaultState {
                    full_range_position_id,
                    base_position_id,
                    limit_position_id
                } = VAULT_STATE.load(deps.storage)?;

                use PositionBalanceComputationError::*;
                let pos_id_to_balances = |id| -> Result<_, PositionBalanceComputationError> { 
                    unimplemented!()
                };

                let compute_all_balances = || {
                    // Invariant: None of those calls will return `None`. If the position
                    //            ids are none, then `pos_id_to_balances` returns a default
                    //            value, and if not, the function can only fail if the position
                    //            ids do not refer to valid positions, but because our position
                    //            ids are in the state, we already verified theyre proper.
                    let full_range_balances = pos_id_to_balances(full_range_position_id); 
                    let base_balances = pos_id_to_balances(base_position_id); 
                    let limit_balances = pos_id_to_balances(limit_position_id);

                    match full_range_balances.err().or(base_balances.err()).or(limit_balances.err()) {
                        None => (),
                        Some(NonExistentPosition(_)) => unreachable!(),
                        Some(Uint128FromStringConversionError(_)) => unreachable!(),
                        Some(Overflow(_)) => unreachable!(),
                    };
                    
                    let contract_balances = {
                        let balances = BankQuerier::new(&deps.querier);
                        let contract_addr = env.contract.address.to_string();

                        // Invariant: Will never return `None` because we know all args are valid.
                        let contract_balance0 = balances
                            .balance(contract_addr.clone(), denom0.clone())
                            .ok()?.balance?.amount;

                        let contract_balance1 = balances
                            .balance(contract_addr, denom1.clone())
                            .ok()?.balance?.amount;

                        // Invariant: Will never return `None` because we know `amount`s are
                        //            properly formated.
                        (
                            Uint128::from_str(&contract_balance0).ok()?,
                            Uint128::from_str(&contract_balance1).ok()?
                        )
                    };

                    // Invariant: None of those additions will return `None`. Balances 
                    //            could only overflow if the tokens they refer had
                    //            supplies above `Uint128::MAX`, but thats not possible.
                    let balance0 = full_range_balances.0
                        .checked_add(base_balances.0).ok()?
                        .checked_add(limit_balances.0).ok()?
                        .checked_add(contract_balances.0).ok()?;

                    let balance1 = full_range_balances.1
                        .checked_add(base_balances.1).ok()?
                        .checked_add(limit_balances.1).ok()?
                        .checked_add(contract_balances.1).ok()?;

                    Some((balance0, balance1))
                };
                unimplemented!()
            };

            // TODO Handle with a match, make unreachable impossible cases, propagate possible errors.
            let (total0, total1) = total_contract_balances_computation()?;

            if total_supply.is_zero() {
                // Invariant: If there are no shares, then there shouldnt be
                //            any vault tokens for that shares.
                assert!(total0.is_zero() && total1.is_zero());

                Ok((cmp::max(amount0, amount1), amount0, amount1))
            } else if total0.is_zero() {
                // Invariant: If there are shares and there are no tokens
                //            denom0 in the vault, then the shares must
                //            be for the token denom1.
                assert!(!total1.is_zero());

                let shares = Into::<Uint256>::into(amount1)
                    .checked_mul(total_supply.into())?
                    .checked_div(total1.into())?;

                Ok((shares.try_into()?, Uint128::zero(), amount1))
            } else if total1.is_zero() {
                // Invariant: If there are shares and there are no tokens
                //            denom1 in the vault, then the shares must
                //            be for the token denom0.
                assert!(!total0.is_zero());

                let shares = Into::<Uint256>::into(amount0)
                    .checked_mul(total_supply.into())?
                    .checked_div(total0.into())?;

                Ok((shares.try_into()?, amount0, Uint128::zero()))
            } else {
                let (amount0, amount1): (Uint256, Uint256) = (amount0.into(), amount1.into());
                let (total0, total1): (Uint256, Uint256) = (total0.into(), total1.into());

                let cross = cmp::min(
                    amount0.checked_mul(total1)?,
                    amount1.checked_mul(total0)?,
                );

                if cross == Uint256::zero() {
                    return Err(InvalidProportion(InvalidProportionError { 
                        expected_amount0: total0.to_string(),
                        expected_amount1: total1.to_string(),
                        got_amount0: amount0.to_string(), 
                        got_amount1: amount1.to_string()
                    }))
                }

                let amount0_used = cross
                    .checked_sub(Uint256::one())?
                    .checked_div(total1)?
                    .checked_add(Uint256::one())?;

                let amount1_used = cross
                    .checked_sub(Uint256::one())?
                    .checked_div(total0)?
                    .checked_add(Uint256::one())?;

                let shares = cross
                    .checked_mul(total_supply.into())?
                    .checked_div(total0)?
                    .checked_div(total1)?;

                Ok((shares.try_into()?, amount0_used.try_into()?, amount1_used.try_into()?))
            }
        };

        use SharesAndAmountsComputationError::*;
        let (new_shares, amount0_used, amount1_used) = match compute_shares_and_amounts() {
            Ok(x) => x,
            // Invariant: Unreachable.
            // Proof: The only call in the closure that could return a 
            //        `TokenSupplyQueryError(StdError)` is the token total
            //        supply query, but because we initialize `TOKEN_INFO`
            //        with the contract instantiation, we know it should 
            //        always be present.
            Err(TokenSupplyQueryError(_)) => unreachable!(),
            Err(TotalContractBalancesComputation(_)) => unreachable!(), // TODO
            // Invariant: Unreachable.
            // Proof: Multiplications wont overflow because we make sure to lift 
            //        to Uint256 every time we do any. Additions wont overflow,
            //        because for them to overflow the token supplies they refer
            //        to should be above Uint128, but thats not possible. `cross`
            //        subtractions wont overflow, becuase before doing any, we
            //        verified `cross > 0` (see `InvalidProportionError`).
            Err(Overflow(_)) => unreachable!(),
            // Invariant: Unreachable.
            // Proof: Every time we divided by zero, we ensured that the denominator
            //        is not zero, either with an assertion, or with an if/else brach.
            Err(DivideByZero(_)) => unreachable!(),
            // Invariant: Unreachable.
            // Proof: We convert back token amounts to `Uint128` after lifting them
            //        to `Uint256` for secure multiplication. But for every
            //        multiplication done, we also did an proportional division 
            //        that puts the amounts back into the expected types.
            Err(ConversionOverflow(_)) => unreachable!(),
            Err(InvalidProportion(err)) => Err(DepositError::InvalidProportion(err))?
        };

        // TODO Whats `MINIMUM_LIQUIDITY`? Probably some hack to prevent weird divisions by 0.
        let (new_shares, amount0_used, amount1_used) = {
            let total_supply = query_token_info(deps.as_ref())?.total_supply;

            // TODO: Test if its possible to manipulate balances by depositing tokens raw to the vault.
            let (total0, total1) = {
                // Invariant: `VAULT_STATE` should always be present after instantiation.
                let VaultState {
                    full_range_position_id,
                    base_position_id,
                    limit_position_id
                } = VAULT_STATE.load(deps.storage).unwrap();

                let pos_id_to_balances = |id| if let Some(id) = id {
                    use osmosis_std::types::cosmos::base::v1beta1::Coin;

                    let pos = PositionByIdRequest { position_id: id }
                        .query(&deps.querier).ok()?.position?;

                    if let FullPositionBreakdown {
                        asset0: Some(Coin { denom: denom0, amount: amount0 }),
                        asset1: Some(Coin { denom: denom1, amount: amount1 }),
                        claimable_spread_rewards: rewards,
                        ..
                    } = pos {

                        // Invariant: Will never return `None`, because if an amount is
                        //            present, it always is a valid `Uint128`.
                        let rewards0 = rewards.iter()
                            .find(|x| x.denom == denom0)
                            .map(|x| Uint128::from_str(&x.amount))
                            .unwrap_or(Ok(Uint128::zero())).ok()?;

                        let rewards1 = rewards.iter()
                            .find(|x| x.denom == denom1)
                            .map(|x| Uint128::from_str(&x.amount))
                            .unwrap_or(Ok(Uint128::zero())).ok()?;


                        // Invariant: Will never return `None`, because if the position has
                        //            amounts `amount0` and `amount1`, we know theyre valid
                        //            `Uint128`. Neither will the addition overflow, because
                        //            balances could only overflow if the tokens they refer
                        //            had supplies above `Uint128::MAX`, but thats not possible.
                        let amount0 = Uint128::from_str(&amount0).ok()?.checked_add(rewards0).ok()?;
                        let amount1 = Uint128::from_str(&amount1).ok()?.checked_add(rewards1).ok()?;
                        Some((amount0, amount1))
                    } else { 
                        /* Invariant: */ unreachable!() 
                        // Proof: If `id` does not refer to a valid position id, we already
                        //        returned `None` at the query at the beggining of the closure.
                        //        Otherwise, any valid position will hold `asset0`, `asset1`,
                        //        and `claimable_spread_rewards` (even if its `vec![]`).
                    }
                } else { Some((Uint128::zero(), Uint128::zero())) };

                let compute_all_balances = || {
                    // Invariant: None of those calls will return `None`. If the position
                    //            ids are none, then `pos_id_to_balances` returns a default
                    //            value, and if not, the function can only fail if the position
                    //            ids do not refer to valid positions, but because our position
                    //            ids are in the state, we already verified theyre proper.
                    let full_range_balances = pos_id_to_balances(full_range_position_id)?; 
                    let base_balances = pos_id_to_balances(base_position_id)?; 
                    let limit_balances = pos_id_to_balances(limit_position_id)?;
                    
                    let contract_balances = {
                        let balances = BankQuerier::new(&deps.querier);
                        let contract_addr = env.contract.address.to_string();

                        // Invariant: Will never return `None` because we know all args are valid.
                        let contract_balance0 = balances
                            .balance(contract_addr.clone(), denom0.clone())
                            .ok()?.balance?.amount;

                        let contract_balance1 = balances
                            .balance(contract_addr, denom1.clone())
                            .ok()?.balance?.amount;

                        // Invariant: Will never return `None` because we know `amount`s are
                        //            properly formated.
                        (
                            Uint128::from_str(&contract_balance0).ok()?,
                            Uint128::from_str(&contract_balance1).ok()?
                        )
                    };

                    // Invariant: None of those additions will return `None`. Balances 
                    //            could only overflow if the tokens they refer had
                    //            supplies above `Uint128::MAX`, but thats not possible.
                    let balance0 = full_range_balances.0
                        .checked_add(base_balances.0).ok()?
                        .checked_add(limit_balances.0).ok()?
                        .checked_add(contract_balances.0).ok()?;

                    let balance1 = full_range_balances.1
                        .checked_add(base_balances.1).ok()?
                        .checked_add(limit_balances.1).ok()?
                        .checked_add(contract_balances.1).ok()?;

                    Some((balance0, balance1))
                };

                // Invariant: Will never return `None`. 
                // Proof: See `compute_all_balances` closure.
                compute_all_balances().unwrap();
                unimplemented!()
            };
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
            base_factor, limit_factor, full_range_weight
        } = vault_parameters;

        // We take 1% slippage.
        let slippage = Weight::new("0.99".to_string()).unwrap();

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

