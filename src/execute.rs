use std::str::FromStr;

use cosmwasm_std::{coin, BankMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Response, StdResult, SubMsg, Uint128};
use cw20_base::{contract::{execute_burn, execute_mint, query_balance, query_token_info}, state::TOKEN_INFO};
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::{MsgCollectSpreadRewards, MsgCreatePosition, MsgWithdrawPosition, PositionByIdRequest};

use crate::{
    constants::{MIN_LIQUIDITY, PROTOCOL, VAULT_CREATION_COST_DENOM}, do_some, error::{AdminOperationError, DepositError, ProtocolOperationError, RebalanceError, WithdrawalError}, msg::{CalcSharesAndUsableAmountsResponse, DepositMsg, VaultBalancesResponse, VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, WithdrawMsg}, query, state::{
        FundsInfo, PositionType, StateSnapshot, VaultInfo, VaultParameters, VaultRebalancer, VaultState, Weight, FEES_INFO, FUNDS_INFO, VAULT_INFO, VAULT_PARAMETERS, VAULT_STATE}, utils::{calc_x0, price_function_inv, raw}};

pub fn deposit(
    DepositMsg {
        amount0,
        amount1,
        amount0_min,
        amount1_min,
        to,
    }: DepositMsg,
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, DepositError> {
    use DepositError::*;
    // Invariant: `VAULT_INFO` will always be present after instantiation.
    let vault_info = VAULT_INFO.load(deps.storage).unwrap();
    let contract_addr = env.contract.address.clone();

    let (denom0, denom1) = vault_info.denoms(&deps.querier);

    if amount0.is_zero() && amount1.is_zero() && info.funds.is_empty() {
        return Err(ZeroTokensSent {});
    }

    let amount0_got = info
        .funds
        .iter()
        .find(|x| x.denom == denom0)
        .map(|x| x.amount)
        .unwrap_or(Uint128::zero());

    let amount1_got = info
        .funds
        .iter()
        .find(|x| x.denom == denom1)
        .map(|x| x.amount)
        .unwrap_or(Uint128::zero());

    if amount0_got != amount0 || amount1_got != amount1 {
        return Err(ImproperSentAmounts {
            expected: format!("({}, {})", amount0, amount1),
            got: format!("({}, {})", amount0_got, amount1_got),
        });
    }

    let new_holder = deps
        .api
        .addr_validate(&to)
        .map_err(|_| InvalidShareholderAddress(to))?;

    if new_holder == contract_addr {
        return Err(ShareholderCantBeContract(new_holder.into()));
    }

    if !(amount0 > MIN_LIQUIDITY || amount1 > MIN_LIQUIDITY) {
        return Err(DepositedAmountBelowMinLiquidity { 
            min_liquidity: MIN_LIQUIDITY.into(),
            got: format!("({}, {})", amount0, amount1)
        })
    }

    let CalcSharesAndUsableAmountsResponse {
        shares,
        usable_amount0: amount0_used,
        usable_amount1: amount1_used,
    } = query::calc_shares_and_usable_amounts(amount0, amount1, deps.as_ref());

    // Invariant: Wont overflow, as for that token balances would have to be above
    //            `Uint128::MAX`, but thats not possible.
    // NOTE: The update is sound as we refund unusued amounts later.
    FUNDS_INFO.update(deps.storage, |mut funds| -> StdResult<_>  {
        funds.available_balance0 = funds.available_balance0.checked_add(amount0_used)?;
        funds.available_balance1 = funds.available_balance1.checked_add(amount1_used)?;
        Ok(funds)
    }).unwrap();

    // Invariant: We already verified the inputed amounts are not zero, 
    //            thus the resulting shares can never be zero.
    assert!(!shares.is_zero());

    if amount0_used < amount0_min || amount1_used < amount1_min {
        return Err(DepositedAmountsBelowMin {
            used: format!("({}, {})", amount0_used, amount1_used),
            wanted: format!("({}, {})", amount0_min, amount1_min),
        });
    }

    let res = {
        let mut info = info.clone();
        let mut deps = deps;
        info.sender = contract_addr;

        // Invariant: Any state is present after initialization.
        let total_supply = TOKEN_INFO.load(deps.storage).unwrap().total_supply;

        // Invariant: Wont panic, as the only allowed minter is this contract itself,
        let min_mint = if total_supply.is_zero() {
            execute_mint(
                deps.branch(),
                env.clone(),
                info.clone(),
                info.sender.clone().into(),
                MIN_LIQUIDITY
            ).unwrap()
        } else { Response::new() };

        let user_mint = execute_mint(deps, env, info, new_holder.to_string(), shares).unwrap();
        min_mint.add_attributes(user_mint.attributes)
    };

    // Invariant: Share calculation should will never produce usable amounts 
    //            above actual inputed amounts.
    assert!(amount0_used <= amount0 && amount1_used <= amount1);

    // Invariant: Wont panic because of the invariant above.
    Ok(res.add_message(BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: vec![
            coin(amount0.checked_sub(amount0_used).unwrap().into(), denom0),
            coin(amount1.checked_sub(amount1_used).unwrap().into(), denom1)
        ].into_iter().filter(|x| !x.amount.is_zero()).collect()
    }))
}

pub fn rebalance(deps_mut: DepsMut, env: Env, info: MessageInfo) -> Result<Response, RebalanceError> {
    use RebalanceError::*;

    let deps = deps_mut.as_ref();

    // Invariant: Any state will be initialized after instantation.
    let vault_info = VAULT_INFO.load(deps.storage).unwrap();
    let mut vault_state = VAULT_STATE.load(deps.storage).unwrap();

    let pool_id = vault_info.pool_id.clone();
    let price = pool_id.price(&deps.querier);

    can_rebalance(deps, env.clone(), info)?;

    // NOTE: We always update `LastPriceAndTimestamp` even if theyre not used, for
    //       semantical simplicity of the variable.
    vault_state.last_price_and_timestamp = Some(StateSnapshot {
        last_price: price,
        last_timestamp: env.block.time
    });

    let VaultParameters {
        base_factor,
        limit_factor,
        full_range_weight,
    } = VAULT_PARAMETERS.load(deps.storage).unwrap();

    let VaultBalancesResponse { 
        bal0,
        bal1,
        protocol_unclaimed_fees0,
        protocol_unclaimed_fees1,
        admin_unclaimed_fees0,
        admin_unclaimed_fees1
    } = query::vault_balances(deps);

    if bal0.is_zero() && bal1.is_zero() {
        return Err(NothingToRebalance {});
    }

    if price.is_zero() {
        // TODO: If the pool has no price, we should be able to deposit 
        //       in any proportion. But we dont support that for now.
        return Err(PoolWithoutPrice(pool_id.0));
    }

    let (balanced_balance0, balanced_balance1) = {
        let bal0 = Decimal::new(bal0);
        let bal1 = Decimal::new(bal1);

        // Invariant: Wont overflow.
        // Proof: Let `x = bal0` and `y = bal1`. Let `p = Y/X = price`. For the first unwrap
        //        to panic, `p` must be really low, in which case `X` is large and `Y` is
        //        small, thus token `Y` is more scarce, and so the amount `y` will be
        //        proportionally lower. The same reasoning applies to the second unwrap.
        //        If both `Y` and `X` were large, then the price would converge close to `1`,
        //        making both operations equally safe.
        let balanced0 = bal1.checked_div(price).unwrap();
        let balanced1 = bal0.checked_mul(price).unwrap();

        if balanced0 > bal0 {
            (bal0, balanced1)
        } else {
            (balanced0, bal1)
        }
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
    } else {
        assert!(!balanced_balance0.is_zero() && !balanced_balance1.is_zero());
        assert!(!bal0.is_zero() && !bal1.is_zero());

        // We take 0.3% slippage to check if balances have the right proportion.
        let balances_price = balanced_balance1 / balanced_balance0;
        assert!(balances_price >= price * Decimal::from_str("0.997").unwrap());
        assert!(balances_price <= price * Decimal::from_str("1.003").unwrap());
    }

    let (full_range_balance0, full_range_balance1) = {
        let x0 = calc_x0(&base_factor, &full_range_weight, balanced_balance0);
        // Invariant: Wont overflow.
        // Proof: Same reasoning as the proof for x0 computation.
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
        assert!(!full_range_balance0.is_zero() && !full_range_balance1.is_zero());

        // We take 0.3% slippage to check if balances have the right proportion.
        let balances_price = full_range_balance1 / full_range_balance0;
        assert!(balances_price >= price * Decimal::from_str("0.997").unwrap());
        assert!(balances_price <= price * Decimal::from_str("1.003").unwrap())
    }

    let (base_range_balance0, base_range_balance1) = if !base_factor.is_one() {
        // Invariant: Wont overflow, because full range balances will always be
        //            lower than the total balanced balances (see `calc_x0`).
        let base_range_balance0 = balanced_balance0.checked_sub(full_range_balance0).unwrap();
        let base_range_balance1 = balanced_balance1.checked_sub(full_range_balance1).unwrap();

        (base_range_balance0, base_range_balance1)
    } else {
        (Decimal::zero(), Decimal::zero())
    };

    if !base_factor.is_one() && !balanced_balance0.is_zero() {
        assert!(!base_range_balance0.is_zero() && !base_range_balance1.is_zero());
    }

    let (limit_balance0, limit_balance1) = {
        // Invariant: Wont overflow because `bal >= balanced_balance`, as we earlier checked.
        let limit_balance0 = Decimal::new(bal0).checked_sub(balanced_balance0).unwrap();
        let limit_balance1 = Decimal::new(bal1).checked_sub(balanced_balance1).unwrap();
        (limit_balance0, limit_balance1)
    };

    let mut new_position_msgs: Vec<SubMsg> = vec![];

    // If `full_range_balance0` is not zero, we already checked that neither
    // `full_range_balance1` will be. If they happened to be zero, it means that
    // the vault only holds tokens for limit orders for now, or that
    // the vault simply has zero `full_range_weight`.
    if !full_range_weight.is_zero() && !full_range_balance0.is_zero() {
        let lower_tick = vault_info.min_valid_tick(&deps.querier);
        let upper_tick = vault_info.max_valid_tick(&deps.querier);

        new_position_msgs.push(SubMsg::reply_on_success(
            create_position_msg(
                lower_tick,
                upper_tick,
                full_range_balance0,
                full_range_balance1,
                deps,
                &env,
            ),
            0,
        ))
    }

    // We just checked that if `base_range_balance0` is not zero, neither
    // `base_range_balance1` will be.
    if !base_factor.is_one() && !base_range_balance0.is_zero() {
        // Invariant: `base_factor > 1`, thus wont panic.
        let lower_price = price.checked_div(base_factor.0).unwrap();
        let upper_price = price.checked_mul(base_factor.0).unwrap_or(Decimal::MAX);

        let lower_tick = price_function_inv(&lower_price);
        let upper_tick = price_function_inv(&upper_price);

        new_position_msgs.push(SubMsg::reply_on_success(
            create_position_msg(
                lower_tick,
                upper_tick,
                base_range_balance0,
                base_range_balance1,
                deps,
                &env,
            ),
            1,
        ))
    }

    if !limit_factor.is_one() && (!limit_balance0.is_zero() || !limit_balance1.is_zero()) {
        if limit_balance0.is_zero() {
            // Invariant: `limit_factor > 1`, thus wont panic.
            let lower_price = price.checked_div(limit_factor.0).unwrap();
            let lower_tick = price_function_inv(&lower_price);

            // Invariant: Ticks nor Ticks spacings will ever be large enough to
            //            overflow out of `i32`.
            let upper_tick = vault_info
                .current_tick(&deps.querier)
                .checked_sub(vault_info.tick_spacing(&deps.querier))
                .unwrap();

            new_position_msgs.push(SubMsg::reply_on_success(
                create_position_msg(
                    lower_tick,
                    upper_tick,
                    Decimal::zero(),
                    limit_balance1,
                    deps,
                    &env,
                ),
                2,
            ))
        } else if limit_balance1.is_zero() {
            let upper_price = price.checked_mul(limit_factor.0).unwrap_or(Decimal::MAX);
            let upper_tick = price_function_inv(&upper_price);

            // Invariant: Ticks nor Ticks spacings will never be large enough to
            //            overflow out of `i32`.
            let lower_tick = vault_info
                .current_tick(&deps.querier)
                .checked_add(vault_info.tick_spacing(&deps.querier))
                .unwrap();

            new_position_msgs.push(SubMsg::reply_on_success(
                create_position_msg(
                    lower_tick,
                    upper_tick,
                    limit_balance0,
                    Decimal::zero(),
                    deps,
                    &env,
                ),
                2,
            ))
        } else {
            // Invariant: Both limit balances cant be non zero, or the resutling position
            //            wouldnt be a limit position.
            unreachable!()
        }
    }

    let liquidity_removal_msgs: Vec<_> = vec![
        remove_liquidity_msg(PositionType::FullRange, deps, &env, &Weight::max()),
        remove_liquidity_msg(PositionType::Base, deps, &env, &Weight::max()),
        remove_liquidity_msg(PositionType::Limit, deps, &env, &Weight::max()),
    ].into_iter().flatten().collect();

    // Invariant: Wont panic as all types are proper.
    VAULT_STATE.save(deps_mut.storage, &VaultState { 
        last_price_and_timestamp: vault_state.last_price_and_timestamp,
        ..VaultState::default()
    }).unwrap();

    FUNDS_INFO.update(deps_mut.storage, |_| -> StdResult<_> {
        Ok(FundsInfo::default())
    }).unwrap();

    // Invariant: Any addition of tokens wont overflow, because for that the token
    //            max supply would have to be above `Uint128::MAX`, but thats impossible.
    FEES_INFO.update(deps_mut.storage, |mut info| -> StdResult<_> { 
        info.protocol_tokens0_owned = info.protocol_tokens0_owned
            .checked_add(protocol_unclaimed_fees0)?;
        info.protocol_tokens1_owned = info.protocol_tokens1_owned
            .checked_add(protocol_unclaimed_fees1)?;
        info.admin_tokens0_owned = info.admin_tokens0_owned
            .checked_add(admin_unclaimed_fees0)?;
        info.admin_tokens1_owned = info.admin_tokens1_owned
            .checked_add(admin_unclaimed_fees1)?;
        Ok(info)
    }).unwrap();

    let position_ids = liquidity_removal_msgs
        .iter()
        .map(|msg| msg.position_id)
        .collect();

    let rewards_claim_msg = MsgCollectSpreadRewards {
        position_ids,
        sender: env.contract.address.into(),
    };

    Ok(Response::new()
        .add_message(rewards_claim_msg)
        .add_messages(liquidity_removal_msgs)
        .add_submessages(new_position_msgs)
    )
}

fn can_rebalance(deps: Deps, env: Env, info: MessageInfo) -> Result<(), RebalanceError> {
    use RebalanceError::*;
    
    // Invariant: Any state is always present after instantition.
    let vault_info = VAULT_INFO.load(deps.storage).unwrap();
    let vault_state = VAULT_STATE.load(deps.storage).unwrap();
    let price = vault_info.pool_id.price(&deps.querier);
    
    match vault_info.rebalancer {
        VaultRebalancer::Admin { } => {
            // Invariant: The rebalancer cant be `Admin` if admin is not present.
            let admin = vault_info.admin.clone().unwrap();
            if admin != info.sender {
                return Err(UnauthorhizedNonAdminAccount { 
                    admin: admin.into(), got: info.sender.into() 
                })
            }
        },
        VaultRebalancer::Delegate { ref rebalancer } => {
            if rebalancer != info.sender {
                return Err(UnauthorizedDelegateAccount { 
                    delegate: rebalancer.into(), got: info.sender.into() 
                })
            }
        },
        VaultRebalancer::Anyone { 
            ref price_factor_before_rebalance,
            time_before_rabalance 
        } => {
            if let Some(StateSnapshot {
                last_price,
                last_timestamp
            }) = vault_state.last_price_and_timestamp {
                let current_time = env.block.time;
                assert!(current_time > last_timestamp);

                let threshold = last_timestamp.plus_seconds(time_before_rabalance.seconds());
                if threshold > current_time {
                    let time_left = current_time.minus_seconds(threshold.seconds()).seconds();
                    return Err(NotEnoughTimePassed { time_left })
                }

                let upper_bound = last_price
                    .checked_mul(price_factor_before_rebalance.0)
                    .unwrap_or(Decimal::MAX);

                // Invariant: Wont overflow as price factors are always greater or equal to 1
                let lower_bound = last_price
                    .checked_div(price_factor_before_rebalance.0)
                    .unwrap();

                if (lower_bound..=upper_bound).contains(&price) {
                    return Err(PriceHasntMovedEnough { 
                        price: price.to_string(),
                        factor: price_factor_before_rebalance.0.to_string() 
                    })
                }

            }
            
        },
    };
    Ok(())
}

/// # Returns
///
/// - `None`: If `liquidity_proportion == 0` or `for_position` has no open position.
/// - `Some(_)`: Otherwise.
pub fn remove_liquidity_msg(
    for_position: PositionType,
    deps: Deps,
    env: &Env,
    liquidity_proportion: &Weight,
) -> Option<MsgWithdrawPosition> {
    if liquidity_proportion.is_zero() { return None }

    // Invariant: After instantiation, `VAULT_STATE` is always present.
    let position_id = VAULT_STATE
        .load(deps.storage)
        .unwrap()
        .from_position_type(for_position)?;

    // Invariant: We know that if `position_id` is in the state, then
    //            it refers to a valid `FullPositionBreakdown`.
    let position_liquidity = do_some!(PositionByIdRequest { position_id }
        .query(&deps.querier).ok()?
        .position?
        .position?
        .liquidity
    ).unwrap();

    // Invariant: We know any position liquidity is a valid Decimal.
    let position_liquidity = liquidity_proportion
        .mul_dec(&Decimal::from_str(&position_liquidity).unwrap())
        .atomics()
        .to_string();

    Some(MsgWithdrawPosition {
        position_id,
        sender: env.contract.address.clone().into(),
        liquidity_amount: position_liquidity,
    })
}

pub fn create_position_msg(
    lower_tick: i32,
    upper_tick: i32,
    tokens_provided0: Decimal,
    tokens_provided1: Decimal,
    deps: Deps,
    env: &Env,
) -> MsgCreatePosition {
    use osmosis_std::types::cosmos::base::v1beta1::Coin;

    // Invariant: Any state will be initialized after instantation.
    let vault_info = VAULT_INFO.load(deps.storage).unwrap();
    let pool = vault_info.pool(&deps.querier);

    let tokens_provided = vec![
        Coin { denom: pool.token0.clone(), amount: raw(&tokens_provided0) },
        Coin { denom: pool.token1.clone(), amount: raw(&tokens_provided1) },
    ]
    .into_iter()
    .filter(|c| c.amount != "0")
    .collect();

    let lower_tick = vault_info.closest_valid_tick(lower_tick, &deps.querier).into();
    let upper_tick = vault_info.closest_valid_tick(upper_tick, &deps.querier).into();

    let slippage = Weight::new("0.997").unwrap();

    MsgCreatePosition {
        pool_id: pool.id,
        sender: env.contract.address.clone().into(),
        lower_tick,
        upper_tick,
        tokens_provided,
        token_min_amount0: raw(&slippage.mul_dec(&tokens_provided0)),
        token_min_amount1: raw(&slippage.mul_dec(&tokens_provided1)),
    }
}

pub fn withdraw(
    WithdrawMsg {
        shares,
        amount0_min,
        amount1_min,
        to,
    }: WithdrawMsg,
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, WithdrawalError> {
    use WithdrawalError::*;
    if shares.is_zero() { return Err(ZeroSharesWithdrawal {}) }

    let withdrawal_address = deps
        .api
        .addr_validate(&to)
        .map_err(|_| InvalidWithdrawalAddress(to))?;

    if withdrawal_address == env.contract.address {
        return Err(CantWithdrawToContract(withdrawal_address.into()));
    }

    // Invariant: TokenInfo will always be present after instantiation.
    let total_shares_supply = query_token_info(deps.as_ref()).unwrap().total_supply;

    let VaultBalancesResponse { 
        bal0,
        bal1,
        protocol_unclaimed_fees0,
        protocol_unclaimed_fees1,
        admin_unclaimed_fees0,
        admin_unclaimed_fees1
    } = query::vault_balances(deps.as_ref());

    // Invariant: Any addition of tokens wont overflow, because for that the token
    //            max supply would have to be above `Uint128::MAX`, but thats impossible.
    FEES_INFO.update(deps.storage, |mut info| -> StdResult<_> { 
        info.protocol_tokens0_owned = info.protocol_tokens0_owned
            .checked_add(protocol_unclaimed_fees0)?;
        info.protocol_tokens1_owned = info.protocol_tokens1_owned
            .checked_add(protocol_unclaimed_fees1)?;
        info.admin_tokens0_owned = info.admin_tokens0_owned
            .checked_add(admin_unclaimed_fees0)?;
        info.admin_tokens1_owned = info.admin_tokens1_owned
            .checked_add(admin_unclaimed_fees1)?;
        Ok(info)
    }).unwrap();

    // Invariant: We know that `info.sender` is a proper address, thus even if it didnt 
    //            own any shares, the query would return Uint128::zero().
    let shares_held = query_balance(deps.as_ref(), info.sender.clone().into())
        .unwrap()
        .balance;

    let shares_held = Decimal::raw(shares_held.into());
    let total_shares_supply = Decimal::raw(total_shares_supply.into());

    // Invariant: We already verified `total_shares_supply` is not zero,
    //            and we also know that it will always be larger than `shares_held`,
    //            thus the division cant overflow. Also, because the shares will
    //            always be smaller than the total supply, the resulting division
    //            will always be a valid Weight.
    let shares_proportion = Weight::try_from(
        shares_held.checked_div(total_shares_supply).unwrap()
    ).unwrap();

    let expected_withdrawn_amount0 = shares_proportion.mul_raw(bal0).atomics();
    let expected_withdrawn_amount1 = shares_proportion.mul_raw(bal1).atomics();

    // Invariant: Wont underflow as `shares_proportion` is a valid weight.
    FUNDS_INFO.update(deps.storage, |mut funds| -> StdResult<_> {
        funds.available_balance0 = funds.available_balance0.checked_sub(
            shares_proportion.mul_raw(funds.available_balance0).atomics()
        )?;

        funds.available_balance1 = funds.available_balance1.checked_sub(
            shares_proportion.mul_raw(funds.available_balance1).atomics()
        )?;
        Ok(funds)
    }).unwrap();

    if expected_withdrawn_amount0 < amount0_min || expected_withdrawn_amount1 < amount1_min {
        return Err(WithdrawnAmontsBelowMin {
            got: format!(
                "({}, {})",
                expected_withdrawn_amount0, expected_withdrawn_amount1
            ),
            wanted: format!("({}, {})", amount0_min, amount1_min),
        });
    }

    let liquidity_removal_msgs: Vec<_> = vec![
        remove_liquidity_msg(
            PositionType::FullRange,
            deps.as_ref(),
            &env,
            &shares_proportion,
        ),
        remove_liquidity_msg(PositionType::Base, deps.as_ref(), &env, &shares_proportion),
        remove_liquidity_msg(PositionType::Limit, deps.as_ref(), &env, &shares_proportion),
    ]
    .into_iter()
    .flatten()
    .collect();

    if shares_proportion.is_max() {
        VAULT_STATE.update(deps.storage, |x| -> StdResult<_> { Ok(VaultState {
            last_price_and_timestamp: x.last_price_and_timestamp,
            ..VaultState::default()
        })}).unwrap();
    }

    let position_ids = liquidity_removal_msgs
        .iter()
        .map(|msg| msg.position_id)
        .collect();

    let rewards_claim_msg = MsgCollectSpreadRewards {
        position_ids,
        sender: env.contract.address.clone().into(),
    };

    // Invariant: `VAULT_INFO` will always be present after instantiation.
    let (denom0, denom1) = VAULT_INFO.load(deps.storage).unwrap().denoms(&deps.querier);

    let shares_burn_response = execute_burn(deps, env.clone(), info, shares).map_err(|_| {
        InvalidWithdrawalAmount {
            owned: shares_held.atomics().into(),
            withdrawn: shares.into(),
        }
    })?;

    Ok(shares_burn_response
        .add_message(rewards_claim_msg)
        .add_messages(liquidity_removal_msgs)
        .add_message(BankMsg::Send {
            to_address: withdrawal_address.into(),
            amount: vec![
                coin(expected_withdrawn_amount0.into(), denom0),
                coin(expected_withdrawn_amount1.into(), denom1),
            ].into_iter().filter(|c| !c.amount.is_zero()).collect()
        })
    )
}

pub fn withdraw_protocol_fees(deps: DepsMut, info: MessageInfo) -> Result<Response, ProtocolOperationError> {
    // Invariant: Any state is always present after instantiation.
    let mut fees = FEES_INFO.load(deps.storage).unwrap();
    let (denom0, denom1) = VAULT_INFO.load(deps.storage).unwrap().denoms(&deps.querier);
    
    if *PROTOCOL != info.sender {
        return Err(ProtocolOperationError::UnauthorizedProtocolAccount {})
    }

    let tx = BankMsg::Send { 
        to_address: PROTOCOL.to_string(),
        amount: vec![
            coin(fees.protocol_tokens0_owned.into(), denom0),
            coin(fees.protocol_tokens1_owned.into(), denom1),
            coin(fees.protocol_vault_creation_tokens_owned.into(), VAULT_CREATION_COST_DENOM)
        ] 
    };

    fees.protocol_tokens0_owned = Uint128::zero();
    fees.protocol_tokens1_owned = Uint128::zero();
    fees.protocol_vault_creation_tokens_owned = Uint128::zero();

    // Invariant: Will serialize as all types are proper.
    FEES_INFO.save(deps.storage, &fees).unwrap();
    Ok(Response::new().add_message(tx))
}

pub fn withdraw_admin_fees(deps: DepsMut, info: MessageInfo) -> Result<Response, AdminOperationError> {
    use AdminOperationError::*;
    // Invariant: Any state is always present after instantiation.
    let mut fees = FEES_INFO.load(deps.storage).unwrap();
    let vault_info = VAULT_INFO.load(deps.storage).unwrap();
    let (denom0, denom1) = vault_info.denoms(&deps.querier);

    let admin = vault_info.admin
        .ok_or(NonExistantAdmin("withdraw_admin_fees".into()))?;

    if info.sender != admin {
        return Err(UnauthorizedAdminAccount("withdraw_admin_fees".into()))
    }

    let tx = BankMsg::Send { 
        to_address:  admin.into(),
        amount: vec![
            coin(fees.admin_tokens0_owned.into(), denom0),
            coin(fees.admin_tokens1_owned.into(), denom1)
        ] 
    };

    fees.admin_tokens0_owned = Uint128::zero();
    fees.admin_tokens1_owned = Uint128::zero();

    // Invariant: Will serialize as all types are proper.
    FEES_INFO.save(deps.storage, &fees).unwrap();
    Ok(Response::new().add_message(tx))
}

pub fn change_vault_info(
    new_vault_info: VaultInfoInstantiateMsg,
    deps: DepsMut,
    info: MessageInfo
) -> Result<Response, AdminOperationError> {
    use AdminOperationError::*;
    // Invariant: Any state is present after instantiation.
    let current_vault_info = VAULT_INFO.load(deps.storage).unwrap();
    let current_token_info = TOKEN_INFO.load(deps.storage).unwrap();

    let admin = current_vault_info.admin.clone()
        .ok_or(NonExistantAdmin("change_vault_info".into()))?;

    if info.sender != admin {
        return Err(UnauthorizedAdminAccount("change_vault_info".into()))
    }

    if new_vault_info.pool_id != current_vault_info.pool_id.0 {
        return Err(ImmutableReInstantiation("pool_id".into()))
    }

    if new_vault_info.vault_name != current_token_info.name {
        return Err(ImmutableReInstantiation("vault_name".into()))
    }

    if new_vault_info.vault_symbol != current_token_info.symbol {
        return Err(ImmutableReInstantiation("vault_symbol".into()))
    }

    let new_vault_info = VaultInfo::new(new_vault_info, deps.as_ref())?;

    // Invariant: Wont panic as we ensured all types are proper during development.
    VAULT_INFO.save(deps.storage, &new_vault_info).unwrap();
    Ok(Response::new())
}

pub fn change_vault_parameters(
    new_vault_parameters: VaultParametersInstantiateMsg,
    deps: DepsMut,
    info: MessageInfo
) -> Result<Response, AdminOperationError> {
    use AdminOperationError::*;
    // Invariant: Any state is present after instantiation.
    let current_vault_info = VAULT_INFO.load(deps.storage).unwrap();

    let admin = current_vault_info.admin.clone()
        .ok_or(NonExistantAdmin("change_vault_parameters".into()))?;

    if info.sender != admin {
        return Err(UnauthorizedAdminAccount("change_vault_info".into()))
    }

    let new_vault_parameters = VaultParameters::new(new_vault_parameters)?;
    // Invariant: Wont panic as we ensured all types are proper during development.
    VAULT_PARAMETERS.save(deps.storage, &new_vault_parameters).unwrap();
    
    Ok(Response::new())
}

pub fn change_admin_fee(
    new_admin_fee: String,
    deps: DepsMut
) -> Result<Response, AdminOperationError> {
    // Invariant: Any state is present after instantiation.
    let fees_info = FEES_INFO.load(deps.storage).unwrap();
    let new_fees_info = fees_info.update(new_admin_fee, deps.as_ref())?;
    // Invariant: Wont panic as we ensured all types are proper during development.
    FEES_INFO.save(deps.storage, &new_fees_info).unwrap();

    Ok(Response::new())
}
