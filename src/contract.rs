use cosmwasm_std::{coins, entry_point, from_binary, from_json, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Reply, Response, StdError, StdResult, Uint128};
use cw20_base::contract::{execute_mint, query_balance};
use cw20_base::state::{MinterData, TokenInfo, TOKEN_INFO};
use cw_utils::{parse_execute_response_data, parse_reply_execute_data};
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::MsgCreatePositionResponse;
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
    env: Env,
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
        mint: Some(MinterData { minter: env.contract.address, cap: None })
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
            Ok(to_json_binary(&query::calc_shares_and_usable_amounts(for_amount0, for_amount1, false, deps, &env))?)
        },
        Balance { address } => to_json_binary(&query_balance(deps, address)?)
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
        amounts_already_in_contract: bool,
        deps: Deps,
        env: &Env
    ) -> CalcSharesAndUsableAmountsResponse {

        let VaultBalancesResponse { 
            bal0: total0,
            bal1: total1
        } = query::vault_balances(deps, env);

        let (total0, total1) = if amounts_already_in_contract {(
            total0.checked_sub(input_amount0).unwrap(),
            total1.checked_sub(input_amount1).unwrap()
        )} else {(total0, total1)};

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
    use cosmwasm_std::{Decimal, SubMsg};
    use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::{
        MsgCreatePosition, MsgWithdrawPosition, PositionByIdRequest
    };
    use crate::{msg::{DepositMsg, PositionType}, state::{price_function_inv, raw, Weight}};
    use super::*;

    // TODO More clarifying errors. TODO Events to query positions (deposits).
    pub fn deposit(
        DepositMsg { amount0, amount1, amount0_min, amount1_min, to }: DepositMsg,
        deps: DepsMut,
        env: Env,
        info: MessageInfo
    ) -> Result<Response, ContractError> {
        use cosmwasm_std::{BankMsg, Uint128};
        
        // Invariant: `VAULT_INFO` should always be present after instantiation.
        let vault_info = VAULT_INFO.load(deps.storage).unwrap();
        let contract_addr = env.contract.address.clone();

        let denom0 = vault_info.demon0(&deps.querier);
        let denom1 = vault_info.demon1(&deps.querier);

        // TODO: Handle better Errors: something like `use errors::DepositErrors::*`.
        let amount0 = Uint128::from_str(&amount0).map_err(|_| ContractError::NonUint128CoinAmount(amount0))?;
        let amount1 = Uint128::from_str(&amount1).map_err(|_| ContractError::NonUint128CoinAmount(amount1))?;
        let amount0_min = Uint128::from_str(&amount0_min).map_err(|_| ContractError::NonUint128CoinAmount(amount0_min))?;
        let amount1_min = Uint128::from_str(&amount1_min).map_err(|_| ContractError::NonUint128CoinAmount(amount1_min))?;

        if amount0.is_zero() && amount1.is_zero() && info.funds.is_empty() {
            return Err(ContractError::ZeroTokensSent {})
        }

        let amount0_got = info.funds.iter()
            .find(|x| x.denom == denom0)
            .map(|x| x.amount)
            .unwrap_or(Uint128::zero());

        let amount1_got = info.funds.iter()
            .find(|x| x.denom == denom1)
            .map(|x| x.amount)
            .unwrap_or(Uint128::zero());

        if amount0_got != amount0 || amount1_got != amount1 {
            return Err(ContractError::ImproperSentAmounts { 
                expected: format!( "({}, {})", amount0, amount1),
                got: format!("({}, {})", amount0_got, amount1_got)
            })
        }

        let new_holder = deps.api.addr_validate(&to)
            .map_err(|_| ContractError::InvalidShareholderAddress(to))?;

        if new_holder == contract_addr {
            return Err(ContractError::ImproperSharesOwner(new_holder.into()))
        }

        let CalcSharesAndUsableAmountsResponse {
            shares, 
            usable_amount0: amount0_used,
            usable_amount1: amount1_used
        } = query::calc_shares_and_usable_amounts(amount0, amount1, true, deps.as_ref(), &env);


        // TODO Whats `MINIMUM_LIQUIDITY`? Probably some hack to prevent weird divisions by 0.

        // TODO: Document those invariants. THEYRE NOT requirements, even if it looks like i it.
        assert!(amount0_used <= amount0 && amount1_used <= amount1);
        assert!(!shares.is_zero());

        if amount0_used < amount0_min || amount1_used < amount1_min {
            return Err(ContractError::DepositedAmontsBelowMin { 
                used: format!("({}, {})", amount0_used, amount1_used),
                wanted: format!("({}, {})", amount0_min, amount1_min)
            })
        }

        let res = {
            let mut info = info.clone();
            info.sender = contract_addr;

            // Invariant: The only allowed minter is this contract itself.
            execute_mint(deps, env, info, new_holder.to_string(), shares).unwrap()
        };
        
        // TODO Clean this procedure. Is the problem zero amounts? Cant I send 2 amounts at the
        // same time? maybe by filtering the original vec I had?

        let res = if amount0_used < amount0 {
            // Invariant: We just verified the subtraction wont overflow.
            res.add_message(BankMsg::Send { 
                to_address: info.sender.to_string(), 
                amount: coins(amount0.checked_sub(amount0_used).unwrap().into(), denom0)
            })
        } else { res };

        let res = if amount1_used < amount1 {
            // Invariant: We just verified the subtraction wont overflow.
            res.add_message(BankMsg::Send { 
                to_address: info.sender.to_string(), 
                amount: coins(amount1.checked_sub(amount1_used).unwrap().into(), denom1)
            })
        } else { res };

        Ok(res)
    }

    // TODO Better return type. TODO hmmmm think of the messages structure.
    pub fn remove_liquidity_msg(for_position: PositionType, deps: Deps, env: &Env) -> Option<MsgWithdrawPosition> {
        // Invariant: After instantiation, `VAULT_STATE` is always present.
        let VaultState { 
            full_range_position_id,
            base_position_id, 
            limit_position_id 
        } = VAULT_STATE.load(deps.storage).unwrap();

        use PositionType::*;
        let position_id = match for_position {
            FullRange => full_range_position_id,
            Base => base_position_id,
            Limit => limit_position_id
        }?;

        // Invariant: We know that if `position_id` is in the state, then
        //            it refers to a valid `FullPositionBreakdown`.
        let position_liquidity = PositionByIdRequest { position_id }
            .query(&deps.querier).unwrap()
            .position.unwrap()
            .position.unwrap()
            .liquidity;

        // USE THIS!! (3 different ids). https://github.com/CosmWasm/cosmwasm/blob/main/SEMANTICS.md#handling-the-reply
        // Response::new().add_submessage(SubMsg::reply_on_success(msg, id));
        // TODO Also have to claim spread factor manually.
        Some(MsgWithdrawPosition {
            position_id,
            sender: env.contract.address.clone().into(),
            liquidity_amount: position_liquidity
        })
    }

    pub fn create_position_msg(
        lower_tick: i32,
        upper_tick: i32,
        tokens_provided0: Decimal,
        tokens_provided1: Decimal,
        deps: Deps,
        env: &Env
    ) -> MsgCreatePosition {
        use osmosis_std::types::cosmos::base::v1beta1::Coin;

        // Invariant: Any state will be initialized after instantation.
        let vault_info = VAULT_INFO.load(deps.storage).unwrap();
        let pool_id = &vault_info.pool_id;
        let pool = pool_id.to_pool(&deps.querier);

        let tokens_provided = vec![
            Coin { denom: pool.token0.clone(), amount: raw(&tokens_provided0) },
            Coin { denom: pool.token1.clone(), amount: raw(&tokens_provided1) }
        ];

        let lower_tick = pool_id.closest_valid_tick(lower_tick, &deps.querier).into();
        let upper_tick = pool_id.closest_valid_tick(upper_tick, &deps.querier).into();

        // We take 1% slippage. 
        // TODO It shouldnt be needed, test rebalances without slippage.
        let slippage = Weight::new(&"0.99".into()).unwrap();

        MsgCreatePosition {
            pool_id: pool.id,  
            sender: env.contract.address.clone().into(),
            lower_tick,
            upper_tick,
            tokens_provided,
            token_min_amount0: raw(&slippage.mul_dec(&tokens_provided0)),
            token_min_amount1: raw(&slippage.mul_dec(&tokens_provided1))
        }
    }

    // TODO Finish cleanup.
    pub fn rebalance(deps: Deps, env: Env) -> Result<Response, ContractError> {
        // TODO Can rebalance? Check `VaultRebalancer` and other params,
        // like `minTickMove` or `period`.

        // Invariant: Any state will be initialized after instantation.
        let pool_id = &VAULT_INFO.load(deps.storage).unwrap().pool_id;
        let VaultParameters { 
            base_factor,
            limit_factor,
            full_range_weight 
        } = VAULT_PARAMETERS.load(deps.storage).unwrap();

        let VaultBalancesResponse { bal0, bal1 } = query::vault_balances(deps, &env);

        if bal0.is_zero() && bal1.is_zero() {
            // TODO: Return custom error: Nothing to rebalance.
        }

        let price = pool_id.price(&deps.querier);
        
        let (balanced_balance0, balanced_balance1) = {
            // FIXME Those could overflow under extreme conditions, both the
            // division and the multiplication. Lift to Uint256?
            
            // Assumption: `price` uses 18 decimals. TODO: Prove it! Wtf is "ToLegacyDec()" in the
            // osmosis codebase.
            // TODO Can we downgrade `price` to Uint128 instead?
            let bal0 = Decimal::new(bal0);
            let bal1 = Decimal::new(bal1);

            let balanced0 = bal1.checked_div(price).unwrap();
            let balanced1 = bal0.checked_mul(price).unwrap();

            if balanced0 > bal0 { (bal0, balanced1) } 
            else { (balanced0, bal1) }
        };

        assert!(bal0 >= raw(&balanced_balance0) && bal1 >= raw(&balanced_balance1));

        // Invariant: Balanced positions have both amounts different from zero. 
        //            So, if at least one of the in balance amounts are zero,
        //            then both have to be. And that can only be the case if
        //            at least one of the inputed amounts was also zero, in
        //            which case the inputed amounts could only produce a limit
        //            position.
        if balanced_balance0.is_zero() || balanced_balance1.is_zero() {
            assert!(balanced_balance0.is_zero() && balanced_balance1.is_zero());
            assert!(bal0.is_zero() || bal1.is_zero());
            // TODO: Add invariant to check if balanced amounts are really in
            // proportion (ie, `balanced_balance1/balanced_balance0 = p`)
            // Use slippage if needed.
        }

        let (full_range_balance0, full_range_balance1) = {
            // TODO Document the math (see [[MagmaLiquidity]]).
            // FIXME All those unwraps could fail under extreme conditions. Lift to Uint256?
            let sqrt_k = base_factor.0.sqrt();

            let numerator = full_range_weight
                .mul_dec(&sqrt_k.sqrt())
                .checked_mul(balanced_balance0)
                .ok();

            let denominator = sqrt_k
                .checked_sub(Decimal::one())
                .unwrap() // Invariant: `k` min value is 1, `sqrt(1) - 1 == Decimal::zero()`.
                .checked_add(full_range_weight.0)
                .unwrap(); // Invariant: `w` max value is 1, and we already subtracted 1.

            // Invariant: Wont produce a `DivisionByZero`.
            // Proof: Let `w \in [0, 1]` be the `full_range_weight`. Let `k \in [1, +\infty)`
            //        be the `base_factor`. Then `sqrt(k) + w - 1` could only be `0` if
            //        `sqrt(k) + w` was `1`, but thats impossible, because `w > 0 \lor k > 1`
            //        is invariant (see `VaultParameters` instantiation).
            let x0 = numerator.and_then(|n| n.checked_div(denominator).ok()).unwrap();
            let y0 = x0.checked_mul(price).unwrap();
            (x0, y0)
        };

        // Invariant: If any of the balanced balances is not zero, and if the vault
        //            uses full range positions, then both balances for the full
        //            range position shouldnt be zero, or the resulting position
        //            wouldnt be in proportion.
        if full_range_weight.is_zero() {
            assert!(full_range_balance0.is_zero() && full_range_balance1.is_zero());
        } else if balanced_balance1.is_zero() || balanced_balance0.is_zero() {
            assert!(full_range_balance0.is_zero() && full_range_balance1.is_zero());
        } else {
            assert!(!full_range_balance0.is_zero() && full_range_balance1.is_zero())
            // TODO: Add invariant to check if full range amounts are really in
            // proportion (ie, `full_range_balance1/full_range_balance0 = p`).
            // Use slippage if needed.
        }

        let mut liquidity_removal_msgs: Vec<MsgWithdrawPosition> = vec![];

        remove_liquidity_msg(PositionType::FullRange, deps, &env)
            .map(|msg| liquidity_removal_msgs.push(msg));
        remove_liquidity_msg(PositionType::Base, deps, &env)
            .map(|msg| liquidity_removal_msgs.push(msg));
        remove_liquidity_msg(PositionType::Limit, deps, &env)
            .map(|msg| liquidity_removal_msgs.push(msg));


        let mut new_position_msgs: Vec<SubMsg> = vec![];

        // If `full_range_balance0` is not zero, we already checked that neither
        // `full_range_balance1` will be. If they happened to be zero, it means that
        // the vault only holds tokens for limit orders for now, or that
        // the vault simply has zero `full_range_weight`.
        if !full_range_weight.is_zero() && !full_range_balance0.is_zero() {
            new_position_msgs.push(SubMsg::reply_on_success(create_position_msg(
                pool_id.min_valid_tick(&deps.querier),
                pool_id.max_valid_tick(&deps.querier),
                full_range_balance0,
                full_range_balance1,
                deps,
                &env
            ), 0))
        }


        let (base_range_balance0, base_range_balance1) = if !base_factor.is_one() {
            // TODO Prove that those unwraps will never fail.
            let base_range_balance0 = balanced_balance0
                .checked_sub(full_range_balance0)
                .unwrap();

            let base_range_balance1 = balanced_balance1
                .checked_sub(full_range_balance1)
                .unwrap();

            (base_range_balance0, base_range_balance1)
        } else { (Decimal::one(), Decimal::one()) };

        if !base_factor.is_one() && !balanced_balance0.is_zero() {
            assert!(!base_range_balance0.is_zero() && !base_range_balance1.is_zero());
            // TODO: Add invariant to check if base range amounts are really in
            // proportion (ie, `base_range_balance1/base_range_balance0= p`).
            // Use slippage if needed.
        }

        // We just checked that if `base_range_balance0` is not zero, neither
        // `base_range_balance1` will be.
        if !base_factor.is_one() && !base_range_balance0.is_zero() {

            let current_price = pool_id.price(&deps.querier);
            let lower_price = base_factor.0
                .checked_div(current_price)
                .unwrap_or(Decimal::MIN);

            let upper_price = base_factor.0
                .checked_mul(current_price)
                .unwrap_or(Decimal::MAX);

            new_position_msgs.push(SubMsg::reply_on_success(create_position_msg(
                price_function_inv(&lower_price),
                price_function_inv(&upper_price),
                base_range_balance0,
                base_range_balance1,
                deps,
                &env
            ), 1))
        }

        if !limit_factor.is_one() {
            let (limit_balance0, limit_balance1) = {
                // Invariant: Wont overflow because `bal >= balanced_balance`, as we earlier checked.
                let limit_balance0 = Decimal::new(bal0).checked_sub(balanced_balance0).unwrap();
                let limit_balance1 = Decimal::new(bal1).checked_sub(balanced_balance1).unwrap();
                (limit_balance0, limit_balance1)
            };

            let current_price = pool_id.price(&deps.querier);
            if limit_balance0.is_zero() {
                // TODO Prove computation security.
                let lower_price = current_price
                    .checked_div(current_price)
                    .unwrap_or(Decimal::MIN);

                new_position_msgs.push(SubMsg::reply_on_success(create_position_msg(
                    price_function_inv(&lower_price),
                    pool_id.current_tick(&deps.querier),
                    Decimal::zero(),
                    limit_balance1,
                    deps,
                    &env
                ), 2))
            } else if limit_balance1.is_zero() {
                // TODO Prove computation security.
                let upper_price = current_price
                    .checked_mul(current_price)
                    .unwrap_or(Decimal::MIN);

                new_position_msgs.push(SubMsg::reply_on_success(create_position_msg(
                    pool_id.current_tick(&deps.querier),
                    price_function_inv(&upper_price),
                    limit_balance0,
                    Decimal::zero(),
                    deps,
                    &env
                ), 2))
            } else { 
                // Invariant: Both limit balances cant be zero, or the resutling position
                //            wouldnt be a limit position.
                unreachable!()
            }
        }

        // TODO Callbacks and review the whole thing.
        Ok(Response::new()
            .add_messages(liquidity_removal_msgs)
            .add_submessages(new_position_msgs)
        )
    }
}

// TODO: Prove all unwraps security.
#[entry_point]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    let data = msg.result.unwrap().data.unwrap();
    let new_position: MsgCreatePositionResponse = from_json(&data).unwrap();

    let mut vault_state = VAULT_STATE.load(deps.storage).unwrap();
    match msg.id {
        0 => { vault_state.full_range_position_id = Some(new_position.position_id) },
        1 => { vault_state.base_position_id = Some(new_position.position_id) },
        2 => { vault_state.limit_position_id = Some(new_position.position_id) },
        _ => unreachable!()
    };

    VAULT_STATE.save(deps.storage, &vault_state).unwrap();

    // TODO Add attributes?
    Ok(Response::new())
}
