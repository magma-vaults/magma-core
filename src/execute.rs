use std::str::FromStr;

use cosmwasm_std::{coin, BankMsg, Decimal, Decimal256, Deps, DepsMut, Env, Event, MessageInfo, Response, StdResult, SubMsg, Uint128};
use cw20_base::contract::{execute_burn, execute_mint, query_balance, query_token_info};
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::{MsgCollectSpreadRewards, MsgCreatePosition, MsgWithdrawPosition, PositionByIdRequest};

use crate::{error::{DepositError, RebalanceError, WithdrawalError}, msg::{CalcSharesAndUsableAmountsResponse, DepositMsg, VaultBalancesResponse, WithdrawMsg}, query, state::{PositionType, StateSnapshot, VaultParameters, VaultRebalancer, VaultState, Weight, PROTOCOL_INFO, VAULT_INFO, VAULT_PARAMETERS, VAULT_STATE}, utils::{price_function_inv, raw}};

// TODO More clarifying errors. TODO Events to query positions (deposits).
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

    let CalcSharesAndUsableAmountsResponse {
        shares,
        usable_amount0: amount0_used,
        usable_amount1: amount1_used,
    } = query::calc_shares_and_usable_amounts(amount0, amount1, true, deps.as_ref(), &env);

    // TODO Whats `MINIMUM_LIQUIDITY`? Probably some hack to prevent weird divisions by 0.

    // Invariant: We already verified the inputed amounts are not zero, 
    //            thus the resulting shares can never be zero.
    assert!(!shares.is_zero());

    if amount0_used < amount0_min || amount1_used < amount1_min {
        return Err(DepositedAmontsBelowMin {
            used: format!("({}, {})", amount0_used, amount1_used),
            wanted: format!("({}, {})", amount0_min, amount1_min),
        });
    }

    let res = {
        let mut info = info.clone();
        info.sender = contract_addr;

        // Invariant: The only allowed minter is this contract itself.
        execute_mint(deps, env, info, new_holder.to_string(), shares).unwrap()
    };


    // Invariant: Share calculation should never produce usable amounts 
    //            about actual inputed amounts.
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

// TODO Finish cleanup.
pub fn rebalance(deps_mut: DepsMut, env: Env, info: MessageInfo) -> Result<Response, RebalanceError> {
    use RebalanceError::*;

    let deps = deps_mut.as_ref();

    // Invariant: Any state will be initialized after instantation.
    let vault_info = VAULT_INFO.load(deps.storage).unwrap();
    let mut vault_state = VAULT_STATE.load(deps.storage).unwrap();

    let pool_id = vault_info.pool_id.clone();
    let price = pool_id.price(&deps.querier);

    // TODO Use other params. 
    // TODO Refactor.
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

    let mut events: Vec<Event> = vec![];

    let VaultBalancesResponse { 
        bal0,
        bal1,
        protocol_unclaimed_fees0,
        protocol_unclaimed_fees1 
    } = query::vault_balances(deps, &env);

    events.push(
        Event::new("vault_balances_snapshot")
            .add_attribute("balance0", bal0)
            .add_attribute("balance1", bal1),
    );

    if bal0.is_zero() && bal1.is_zero() {
        return Err(NothingToRebalance {});
    }

    events.push(
        Event::new("vault_pool_price_snapshot").add_attribute("price", price.to_string()),
    );

    if price.is_zero() {
        // TODO: If pool has no price, we can deposit in any proportion.
        return Err(PoolWithoutPrice(pool_id.0));
    }

    let (balanced_balance0, balanced_balance1) = {
        // Assumption: `price` uses 18 decimals. TODO: Prove it! Wtf is "ToLegacyDec()" in the
        // osmosis codebase.
        // TODO Can we downgrade `price` to Uint128 instead?
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

        // We take 1% slippage to check if balances have the right proportion.
        let balances_price = balanced_balance1 / balanced_balance0;
        assert!(balances_price >= price * Decimal::from_str("0.99").unwrap());
        assert!(balances_price <= price * Decimal::from_str("1.01").unwrap());
    }

    // TODO Decouple $x_0, y_0$ computation, as its not trivial.
    let (full_range_balance0, full_range_balance1) = {
        // TODO Document the math (see [[MagmaLiquidity]]).
        // FIXME All those unwraps could fail under extreme conditions. Lift to Uint256?
        // TODO PROVE SECURITY!
        let sqrt_k = base_factor.0.sqrt();

        let numerator = full_range_weight.mul_dec(&sqrt_k);
        // Invariant: Wont overflow because we lifter to 256 bits.
        let numerator = Decimal256::from(numerator)
            .checked_mul(balanced_balance0.into())
            .unwrap();

        let denominator = sqrt_k
            .checked_sub(Decimal::one())
            .unwrap() // Invariant: `k` min value is 1, `sqrt(1) - 1 == Decimal::zero()`.
            .checked_add(full_range_weight.0)
            .unwrap(); // Invariant: `w` max value is 1, and we already subtracted 1.

        // Invariant: Wont produce a `DivisionByZero` nor will overflow.
        // Proof: Let `w \in [0, 1]` be the `full_range_weight`. Let `k \in [1, +\infty)`
        //        be the `base_factor`. Then `sqrt(k) + w - 1` could only be `0` if
        //        `sqrt(k) + w` was `1`, but thats impossible, because `w > 0 \lor k > 1`
        //        is invariant (see `VaultParameters` instantiation). TODO The rest
        //        of the proof is not trivial.
        let x0 = numerator.checked_div(denominator.into()).unwrap();
        let y0 = x0.checked_mul(price.into()).unwrap();
        // Invariant: The downgrade wont overflow.
        // Proof: TODO, not trivial.
        (
            Decimal::try_from(x0).unwrap(),
            Decimal::try_from(y0).unwrap(),
        )
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

        // We take 1% slippage to check if balances have the right proportion.
        let balances_price = full_range_balance1 / full_range_balance0;
        assert!(balances_price >= price * Decimal::from_str("0.99").unwrap());
        assert!(balances_price <= price * Decimal::from_str("1.01").unwrap())
    }

    let (base_range_balance0, base_range_balance1) = if !base_factor.is_one() {
        // Invariant: Wont overflow, because full range balances will always be
        //            lower than the total balanced balances.
        // Proof: TODO, but if we prove that $x_0 < X$, then that also proves
        //        that $x_0$ can be safely downgraded to 128 bits.
        let base_range_balance0 = balanced_balance0.checked_sub(full_range_balance0).unwrap();

        let base_range_balance1 = balanced_balance1.checked_sub(full_range_balance1).unwrap();

        (base_range_balance0, base_range_balance1)
    } else {
        (Decimal::one(), Decimal::one())
    };

    if !base_factor.is_one() && !balanced_balance0.is_zero() {
        assert!(!base_range_balance0.is_zero() && !base_range_balance1.is_zero());

        // We take 1% slippage to check if balances have the right proportion.
        let balances_price = base_range_balance1 / base_range_balance0;
        assert!(balances_price >= price * Decimal::from_str("0.99").unwrap());
        assert!(balances_price <= price * Decimal::from_str("1.01").unwrap())
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

        events.push(
            Event::new("create_vault_position")
                .add_attribute("position_type", "full_range")
                .add_attribute("lower_tick", lower_tick.to_string())
                .add_attribute("upper_tick", upper_tick.to_string())
                .add_attribute("amount0", full_range_balance0.to_string())
                .add_attribute("amount1", full_range_balance1.to_string()),
        );

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

        events.push(
            Event::new("create_vault_position")
                .add_attribute("position_type", "base")
                .add_attribute("lower_tick", lower_tick.to_string())
                .add_attribute("upper_tick", upper_tick.to_string())
                .add_attribute("amount0", base_range_balance0.to_string())
                .add_attribute("amount1", base_range_balance1.to_string()),
        );

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

            events.push(
                Event::new("create_vault_position")
                    .add_attribute("position_type", "limit")
                    .add_attribute("lower_tick", lower_tick.to_string())
                    .add_attribute("upper_tick", upper_tick.to_string())
                    .add_attribute("amount0", limit_balance0.to_string())
                    .add_attribute("amount1", limit_balance1.to_string()),
            );

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

            events.push(
                Event::new("create_vault_position")
                    .add_attribute("position_type", "limit")
                    .add_attribute("lower_tick", lower_tick.to_string())
                    .add_attribute("upper_tick", upper_tick.to_string())
                    .add_attribute("amount0", limit_balance0.to_string())
                    .add_attribute("amount1", limit_balance1.to_string()),
            );

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

    // Invariant: Any addition of tokens wont overflow, because for that the token
    //            max supply would have to be above `Uint128::MAX`, but thats impossible.
    PROTOCOL_INFO.update(deps_mut.storage, |mut info| -> StdResult<_> { 
        info.protocol_tokens0_owned = info.protocol_tokens0_owned
            .checked_add(protocol_unclaimed_fees0)?;
        info.protocol_tokens1_owned = info.protocol_tokens1_owned
            .checked_add(protocol_unclaimed_fees1)?;
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
        .add_events(events)
        .add_message(rewards_claim_msg)
        .add_messages(liquidity_removal_msgs)
        .add_submessages(new_position_msgs)
    )
}

// TODO Test what happens if we remove liquidity with `Weight == 0`.
pub fn remove_liquidity_msg(
    for_position: PositionType,
    deps: Deps,
    env: &Env,
    liquidity_proportion: &Weight,
) -> Option<MsgWithdrawPosition> {
    // Invariant: After instantiation, `VAULT_STATE` is always present.
    let position_id = VAULT_STATE
        .load(deps.storage)
        .unwrap()
        .from_position_type(for_position)?;

    // Invariant: We know that if `position_id` is in the state, then
    //            it refers to a valid `FullPositionBreakdown`.
    let position_liquidity = PositionByIdRequest { position_id }
        .query(&deps.querier)
        .unwrap()
        .position
        .unwrap()
        .position
        .unwrap()
        .liquidity;

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
        Coin {
            denom: pool.token0.clone(),
            amount: raw(&tokens_provided0),
        },
        Coin {
            denom: pool.token1.clone(),
            amount: raw(&tokens_provided1),
        },
    ]
    .into_iter()
    .filter(|c| c.amount != "0")
    .collect();

    let lower_tick = vault_info.closest_valid_tick(lower_tick, &deps.querier).into();
    let upper_tick = vault_info.closest_valid_tick(upper_tick, &deps.querier).into();

    // We take 1% slippage.
    // TODO It shouldnt be needed, test rebalances without slippage.
    let slippage = Weight::new("0.99").unwrap();

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

// TODO Clean function.
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
    if shares.is_zero() {
        return Err(ZeroSharesWithdrawal {});
    }

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
        protocol_unclaimed_fees1 
    } = query::vault_balances(deps.as_ref(), &env);

    // Invariant: Any addition of tokens wont overflow, because for that the token
    //            max supply would have to be above `Uint128::MAX`, but thats impossible.
    PROTOCOL_INFO.update(deps.storage, |mut info| -> StdResult<_> { 
        info.protocol_tokens0_owned = info.protocol_tokens0_owned
            .checked_add(protocol_unclaimed_fees0)?;
        info.protocol_tokens1_owned = info.protocol_tokens1_owned
            .checked_add(protocol_unclaimed_fees1)?;
        Ok(info)
    }).unwrap();

    // Invariant: We know that `info.sender` is a proper address, thus even if it didnt 
    //            any shares, the query would return Uint128::zero().
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

    // Invariant: Wont overflow because we lifted to Uint256. Wont produce a division
    //            by zero error because for shares to exist, the total supply has
    //            to be greater than zero. Wont overflow during Uint128 downgrade because
    //            individual shares will always be smaller than total supply, so the resulting
    //            computation will always be lower than `bal0` or `bal1`.
    // FIXME Adapt this invariant to the Weight lift above.
    let expected_withdrawn_amount0 = shares_proportion.mul_raw(bal0).atomics();
    let expected_withdrawn_amount1 = shares_proportion.mul_raw(bal1).atomics();

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
        InalidWithdrawalAmount {
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
