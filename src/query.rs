use std::{cmp, str::FromStr};

use cosmwasm_std::{Deps, Env, Uint128, Uint256};
use cw20_base::state::TOKEN_INFO;
use osmosis_std::types::{cosmos::bank::v1beta1::BankQuerier, osmosis::concentratedliquidity::v1beta1::PositionByIdRequest};

use crate::{do_me, do_ok, msg::{CalcSharesAndUsableAmountsResponse, PositionBalancesWithFeesResponse, VaultBalancesResponse}, state::{PositionType, PROTOCOL_INFO, VAULT_INFO, VAULT_STATE}};

pub fn vault_balances(deps: Deps, env: &Env) -> VaultBalancesResponse {
    let full_range_balances = position_balances_with_fees(PositionType::FullRange, deps);
    let base_balances = position_balances_with_fees(PositionType::Base, deps);
    let limit_balances = position_balances_with_fees(PositionType::Limit, deps);

    // Invariant: `VAULT_INFO` will always be present after instantiation.
    let (denom0, denom1) = VAULT_INFO
        .load(deps.storage)
        .unwrap()
        .denoms(&deps.querier);

    let contract_addr = env.contract.address.to_string();

    // Invariant: Wont panic becuase we verify the pool and
    //            denoms are proper during instantiation.
    let contract_balance0 = BankQuerier::new(&deps.querier)
        .balance(contract_addr.clone(), denom0.clone()).ok()
        .and_then(|x| x.balance)
        .map(|bal| bal.amount)
        .unwrap();

    let contract_balance1 = BankQuerier::new(&deps.querier)
        .balance(contract_addr, denom1.clone()).ok()
        .and_then(|x| x.balance)
        .map(|bal| bal.amount)
        .unwrap();

    // Invariant: Any state is present after instantiation.
    let protocol_fee = PROTOCOL_INFO.load(deps.storage).unwrap().protocol_fee;

    // Invariant: Wont panic.
    // Proof: Any addition of token amounts wont overflow, because
    //        the max supply of any token is `Uint128::MAX`.
    //        Subtraction of `protocol_unclaimed_fees` wont overflow,
    //        because `protocol_unclaimed_fees < total_token_fees` is
    //        invariant (`protocol_fee.0` is a valid Weight`).
    //        `from_str` conversions wont fail, because we got
    //        `contract_balance` directly from `BankQuerier`, so we
    //        know they refer to valid amounts.
    do_me! { 
        
        let total_token0_fees = full_range_balances.bal0_fees
            .checked_add(base_balances.bal0_fees)?
            .checked_add(limit_balances.bal0_fees)?;

        let total_token1_fees = full_range_balances.bal1_fees
            .checked_add(base_balances.bal1_fees)?
            .checked_add(limit_balances.bal1_fees)?;

        let protocol_unclaimed_fees0 = protocol_fee.0
            .mul_raw(total_token0_fees)
            .atomics();

        let protocol_unclaimed_fees1 = protocol_fee.0
            .mul_raw(total_token1_fees)
            .atomics();

        let bal0 = Uint128::from_str(&contract_balance0)?
            .checked_add(full_range_balances.bal0)?
            .checked_add(base_balances.bal0)?
            .checked_add(limit_balances.bal0)?
            .checked_add(total_token0_fees)?
            .checked_sub(protocol_unclaimed_fees0)?;

        let bal1 = Uint128::from_str(&contract_balance1)?
            .checked_add(full_range_balances.bal1)?
            .checked_add(base_balances.bal1)?
            .checked_add(limit_balances.bal1)?
            .checked_add(total_token1_fees)?
            .checked_sub(protocol_unclaimed_fees1)?;
        
        VaultBalancesResponse { 
            bal0,
            bal1,
            protocol_unclaimed_fees0,
            protocol_unclaimed_fees1
        }
    }.unwrap()
}

pub fn position_balances_with_fees(
    position_type: PositionType,
    deps: Deps,
) -> PositionBalancesWithFeesResponse {

    // Invariant: `VAULT_STATE` will always be present after instantiation.
    let id = VAULT_STATE.load(deps.storage).unwrap().from_position_type(position_type);
    let id = match id {
        None => return PositionBalancesWithFeesResponse::default(),
        Some(id) => id
    };

    // Invariant: We verified `id` is a valid position id the moment
    //            we put it in the state, so the query wont fail.
    let pos = PositionByIdRequest { position_id: id }
        .query(&deps.querier)
        .map(|x| x.position.unwrap())
        .unwrap();

    // Invariant: If position is valid, both assets will be always present.
    let asset0 = pos.asset0.unwrap();
    let asset1 = pos.asset1.unwrap();
    let rewards = pos.claimable_spread_rewards;

    { 
        // Invariant: `VAULT_INFO` will always be present after instantiation.
        let (denom0, denom1) = VAULT_INFO
            .load(deps.storage)
            .unwrap()
            .denoms(&deps.querier);
        assert!(denom0 == asset0.denom && denom1 == asset1.denom);
        // Invariant: If `pos` is a valid position, it will always have a `position_id`.
        assert!(pos.position.unwrap().position_id == id);
    }

    // Invariant: Will never panic, because if the position has amounts
    //            `amount0` and `amount1`, we know theyre valid `Uint128`s.
    // NOTE: We subtract 1 to prevent dust error durign withdrawals, as 
    //       position withdrawals can leave 1 atomic token behind.
    let bal0 = Uint128::from_str(&asset0.amount)
        .unwrap()
        .checked_sub(Uint128::one())
        .unwrap_or(Uint128::zero());

    let bal1 = Uint128::from_str(&asset1.amount)
        .unwrap()
        .checked_sub(Uint128::one())
        .unwrap_or(Uint128::zero());

    // Invariant: If `rewards` is present, we know its a `Vec` of valid
    //            amounts, so the conversion will never fail.
    let rewards0 = rewards
        .iter()
        .find(|x| x.denom == asset0.denom)
        .map(|x| Uint128::from_str(&x.amount))
        .unwrap_or(Ok(Uint128::zero()))
        .unwrap();
    
    let rewards1 = rewards
        .iter()
        .find(|x| x.denom == asset1.denom)
        .map(|x| Uint128::from_str(&x.amount))
        .unwrap_or(Ok(Uint128::zero()))
        .unwrap();

    PositionBalancesWithFeesResponse { 
        bal0,
        bal1,
        bal0_fees: rewards0,
        bal1_fees: rewards1
    }
}

// TODO: Test dust errors for this function.
// TODO FIXME: How can I fix this ugly flag!
// NOTE: I cant just pass a `VaultBalancesResponse` as arg, because
//       that would allow the dev to make undesired mutations to it!
//       But theres only one way of constructing the type: through 
//       the `vault_balances` query, so maybe theres a nicer way of
//       handling Msgs and MsgResponses.
pub fn calc_shares_and_usable_amounts(
    input_amount0: Uint128,
    input_amount1: Uint128,
    amounts_already_in_contract: bool,
    deps: Deps,
    env: &Env,
) -> CalcSharesAndUsableAmountsResponse {
    let VaultBalancesResponse {
        bal0: total0, bal1: total1, ..
    } = vault_balances(deps, env);

    // Invariant: If the amounts are already in the contract, then, in
    //            the worst case, the subtraction will be 0.
    let (total0, total1) = if amounts_already_in_contract {
        (
            total0.checked_sub(input_amount0).unwrap(),
            total1.checked_sub(input_amount1).unwrap(),
        )
    } else {
        (total0, total1)
    };

    // Invariant: `TOKEN_INFO` always present after instantiation.
    let total_supply = TOKEN_INFO.load(deps.storage).unwrap().total_supply;

    if total_supply.is_zero() {
        assert!(total0.is_zero() && total1.is_zero());

        CalcSharesAndUsableAmountsResponse {
            shares: cmp::max(input_amount0, input_amount1),
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
        //            wont fail becuase we just ensured `total1`
        //            is not zero. The downgrade back to `Uint128`
        //            wont fail because we divided proportionally
        //            by `total1`. The same reasoning applies to
        //            the rest of branches.
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

        // Invariant: Wont panic.
        // Proof: TODO.
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
