use std::{cmp, str::FromStr};

use cosmwasm_std::{Deps, Env, Uint128, Uint256};
use cw20_base::state::TOKEN_INFO;
use osmosis_std::types::{cosmos::{bank::v1beta1::BankQuerier, base::v1beta1::Coin}, osmosis::concentratedliquidity::v1beta1::PositionByIdRequest};

use crate::{
    constants::MIN_LIQUIDITY,
    do_me, do_ok,
    msg::{
        CalcSharesAndUsableAmountsResponse, PositionBalancesWithFeesResponse, VaultBalancesResponse,
    },
    state::{PositionType, FEES_INFO, VAULT_INFO, VAULT_STATE},
};

/// Partition available balances to the vault in 3 sets:
/// - Balances available for business logic, e.g., for creating new positions.
/// - Idle protocol fees, not yet claimed nor commited to the state.
/// - Idle vault admin fees, not yet claimed nor commited to the state.
///
/// For this, query the fees and balances in all current vault positions and 
/// the funds in the contract (except for funds tracked by [`FEES_INFO`]).
pub fn vault_balances(deps: Deps, env: Env) -> VaultBalancesResponse {
    let full_range_balances = position_balances_with_fees(PositionType::FullRange, deps);
    let base_balances = position_balances_with_fees(PositionType::Base, deps);
    let limit_balances = position_balances_with_fees(PositionType::Limit, deps);

    let bank = BankQuerier::new(&deps.querier);
    let contract_addr = env.contract.address.as_str();
    // Invariant: Any state will always be present after instantiation.
    let (denom0, denom1) = VAULT_INFO.load(deps.storage).unwrap().denoms(&deps.querier);

    let fees = FEES_INFO.load(deps.storage).unwrap();
    
    // Invariant: Wont Panic.
    // Proof: We know `contract_addr` and `denom(0|1)` are proper. We also know
    //        commited protocol/admin fees partition the vault available balances.
    let (available_balance0, available_balance1) = do_me! {
        let available_balance0 = Uint128::from_str(
            &bank.balance(contract_addr.into(), denom0)?.balance.unwrap().amount
        )?;
        let available_balance1 = Uint128::from_str(
            &bank.balance(contract_addr.into(), denom1)?.balance.unwrap().amount
        )?;

        (
            available_balance0
                .checked_sub(fees.admin_tokens0_owned)?
                .checked_sub(fees.protocol_tokens0_owned)?,
            available_balance1
                .checked_sub(fees.admin_tokens1_owned)?
                .checked_sub(fees.protocol_tokens1_owned)?
        )
    }.unwrap();

    // Invariant: Wont panic.
    // Proof: If the contract has unclaimed fees, we know its balance will at
    //        least be those fees, so the subtractions wont underflow. Any
    //        addition of token amounts wont overflow, because for that the
    //        token supply of any token would have to be above `Uint128::MAX`.
    //        Products wont overflow, as we know the fees are valid weights.
    do_me! { 
        let total_token0_fees = full_range_balances.bal0_fees
            .checked_add(base_balances.bal0_fees)?
            .checked_add(limit_balances.bal0_fees)?;

        let total_token1_fees = full_range_balances.bal1_fees
            .checked_add(base_balances.bal1_fees)?
            .checked_add(limit_balances.bal1_fees)?;

        let protocol_unclaimed_fees0 = fees.protocol_fee.0
            .mul_raw(total_token0_fees)
            .atomics();

        let protocol_unclaimed_fees1 = fees.protocol_fee.0
            .mul_raw(total_token1_fees)
            .atomics();

        let admin_unclaimed_fees0 = fees.admin_fee.0
            .mul_raw(total_token0_fees)
            .atomics();

        let admin_unclaimed_fees1 = fees.admin_fee.0
            .mul_raw(total_token1_fees)
            .atomics();

        let bal0 = available_balance0
            .checked_add(full_range_balances.bal0)?
            .checked_add(base_balances.bal0)?
            .checked_add(limit_balances.bal0)?
            .checked_add(total_token0_fees)?
            .checked_sub(protocol_unclaimed_fees0)?
            .checked_sub(admin_unclaimed_fees0)?;

        let bal1 = available_balance1
            .checked_add(full_range_balances.bal1)?
            .checked_add(base_balances.bal1)?
            .checked_add(limit_balances.bal1)?
            .checked_add(total_token1_fees)?
            .checked_sub(protocol_unclaimed_fees1)?
            .checked_sub(admin_unclaimed_fees1)?;
        
        VaultBalancesResponse { 
            bal0, bal1,
            protocol_unclaimed_fees0, protocol_unclaimed_fees1,
            admin_unclaimed_fees0, admin_unclaimed_fees1
        }
    }.unwrap()
}

pub fn position_balances_with_fees(
    position_type: PositionType,
    deps: Deps,
) -> PositionBalancesWithFeesResponse {
    do_me! {
        // Invariant: `VAULT_STATE` will always be present after instantiation.
        let id = VAULT_STATE.load(deps.storage)?.from_position_type(position_type);
        let id = match id {
            None => return Ok(PositionBalancesWithFeesResponse::default()),
            Some(id) => id
        };

        // Invariant: We verified `id` is a valid position id the moment
        //            we put it in the state, so the query wont fail.
        let pos = PositionByIdRequest { position_id: id }
            .query(&deps.querier)
            .map(|x| x.position.unwrap())?;

        // Invariant: If position is valid, both assets will be always present,
        //            even for out of range positions.
        let asset0 = pos.asset0.unwrap();
        let asset1 = pos.asset1.unwrap();

        let spread_rewards = pos.claimable_spread_rewards;
        let incentive_rewards = pos.claimable_incentives;

        {
            let (denom0, denom1) = VAULT_INFO.load(deps.storage).unwrap().denoms(&deps.querier);
            // Invariant: `VAULT_INFO` will always be present after instantiation.
            assert!(denom0 == asset0.denom && denom1 == asset1.denom);
            // Invariant: If `pos` is a valid position, it will always have a `position_id`.
            assert!(pos.position.unwrap().position_id == id);
        }

        // Invariant: Will never panic, because if the position has amounts
        //            `amount0` and `amount1`, we know theyre valid `Uint128`s.
        // NOTE: We subtract 1 to prevent dust error during withdrawals, as 
        //       position withdrawals can leave 1 atomic token behind.
        let bal0 = Uint128::from_str(&asset0.amount)?
            .checked_sub(Uint128::one())
            .unwrap_or(Uint128::zero());

        let bal1 = Uint128::from_str(&asset1.amount)?
            .checked_sub(Uint128::one())
            .unwrap_or(Uint128::zero());

        let extract_amount = |from_coins: &Vec<Coin>, for_denom: &str| from_coins
            .iter()
            .find(|x| x.denom == for_denom)
            .map(|x| Uint128::from_str(&x.amount))
            .unwrap_or(Ok(Uint128::zero()));

        // Invariant: If `spread_rewards` or `incentive_rewards` is present, we know
        //            its a `Vec` of valid amounts, so the conversions to `Uint128`s 
        //            will never fail.
        let spread_rewards0 = extract_amount(&spread_rewards, &asset0.denom)?;
        let spread_rewards1 = extract_amount(&spread_rewards, &asset1.denom)?;
        let incentive_rewards0 = extract_amount(&incentive_rewards, &asset0.denom)?;
        let incentive_rewards1 = extract_amount(&incentive_rewards, &asset1.denom)?;

        // Invariant: Wont overflow, as for that those tokens would
        //            have to have supply above `Uint128::MAX`.
        let bal0_fees = spread_rewards0.checked_add(incentive_rewards0)?;
        let bal1_fees = spread_rewards1.checked_add(incentive_rewards1)?;

        PositionBalancesWithFeesResponse { bal0, bal1, bal0_fees, bal1_fees }
    }.unwrap()
}

/// # Arguments FIXME WTH!!! Make it a pure effing finction....
///
/// * `input_amount0` - Amount of token0 for which we want to calculate shares for, 
///                     not yet in the contract state ([`FUNDS_INFO`]).
///
/// * `input_amount1` - Amount of token1 for which we want to calculate shares for, 
///                     not yet in the contract state ([`FUNDS_INFO`]).
pub fn calc_shares_and_usable_amounts(
    input_amount0: Uint128,
    input_amount1: Uint128,
    deps: Deps,
    env: Env
) -> CalcSharesAndUsableAmountsResponse {
    let VaultBalancesResponse { bal0: total0, bal1: total1, .. } = vault_balances(deps, env);
    // Invariant: Wont underflow, as `input_amount(0|1)` always included in the current balances
    //            available to the contract.
    let total0 = total0.checked_sub(input_amount0).unwrap();
    let total1 = total1.checked_sub(input_amount1).unwrap();

    // Invariant: `TOKEN_INFO` always present after instantiation.
    let total_supply = TOKEN_INFO.load(deps.storage).unwrap().total_supply;

    if total_supply.is_zero() {
        assert!(total0.is_zero() && total1.is_zero());
        // Invariant: Wont overflow. See [`DepositError::DepositedAmountBelowMinLiquidity`].
        CalcSharesAndUsableAmountsResponse {
            shares: cmp::max(input_amount0, input_amount1)
                .checked_sub(MIN_LIQUIDITY).unwrap(),
            usable_amount0: input_amount0,
            usable_amount1: input_amount1,
        }
    } else if total0.is_zero() {
        // Invariant: If there are shares and there are no tokens
        //            denom0 in the vault, then the shares must
        //            be for the token denom1.
        assert!(!total1.is_zero());

        // Invariant: The multiplication wont overflow becuase we
        //            lifted the amount to `Uint256`. The division
        //            wont panic, becuase we know from above that 
        //            `total1` is not zero. The downgrade back to
        //            `Uint128` wont fail because we divided
        //            proportionally by `total1`. The same
        //            reasoning applies to the rest of branches.
        let shares = do_ok!(Uint256::from(input_amount1)
           .checked_mul(total_supply.into())?
           .checked_div(total1.into())?
           .try_into()?
        ).unwrap();

        CalcSharesAndUsableAmountsResponse {
            shares,
            usable_amount0: Uint128::zero(),
            usable_amount1: input_amount1,
        }
    } else if total1.is_zero() {
        // Invariant: If there are shares and there are no tokens
        //            denom1 in the vault, then the shares must
        //            be for the token denom0.
        assert!(!total0.is_zero());

        let shares = do_ok!(Uint256::from(input_amount0)
            .checked_mul(total_supply.into())?
            .checked_div(total0.into())?
            .try_into()?
        ).unwrap();

        CalcSharesAndUsableAmountsResponse {
            shares,
            usable_amount0: input_amount0,
            usable_amount1: Uint128::zero(),
        }
    } else {
        let input_amount0: Uint256 = input_amount0.into();
        let input_amount1: Uint256 = input_amount1.into();
        let total0: Uint256 = total0.into();
        let total1: Uint256 = total1.into();

        do_me! {
            let cross = cmp::min(
                input_amount0.checked_mul(total1)?,
                input_amount1.checked_mul(total0)?
            );

            if cross.is_zero() {
                return Ok(CalcSharesAndUsableAmountsResponse::default())
            }

            let usable_amount0 = cross
                .checked_sub(Uint256::one())?
                .checked_div(total1)?
                .checked_add(Uint256::one())?
                .try_into()?;

            let usable_amount1 = cross
                .checked_sub(Uint256::one())?
                .checked_div(total0)?
                .checked_add(Uint256::one())?
                .try_into()?;

            let shares = cross
                .checked_mul(total_supply.into())?
                .checked_div(total0)?
                .checked_div(total1)?
                .try_into()?;

            CalcSharesAndUsableAmountsResponse {
                shares,
                usable_amount0,
                usable_amount1,
            }
        }.unwrap()
    }
}

