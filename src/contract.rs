use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Reply, Response,
    StdResult, Storage, Uint128,
};
use cw20_base::contract::query_balance;
use cw20_base::state::{MinterData, TokenInfo, TOKEN_INFO};
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::MsgCreatePositionResponse;
use std::cmp;

use crate::msg::{
    CalcSharesAndUsableAmountsResponse, PositionBalancesWithFeesResponse, QueryMsg,
    VaultBalancesResponse,
};
use crate::{
    error::ContractError,
    msg::{ExecuteMsg, InstantiateMsg},
    state::{VaultInfo, VaultParameters, VaultState, VAULT_INFO, VAULT_PARAMETERS, VAULT_STATE},
};

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    // TODO Better Error types!!!!
    let vault_info = VaultInfo::new(msg.vault_info.clone(), deps.as_ref())?;
    // Invaraint: `VaultInfo` serialization should never fail.
    VAULT_INFO
        .save(deps.storage as &mut dyn Storage, &vault_info)
        .unwrap();

    let vault_parameters = VaultParameters::new(msg.vault_parameters)?;
    // Invariant: `VaultParameters` serialization should never fail.
    VAULT_PARAMETERS
        .save(deps.storage, &vault_parameters)
        .unwrap();

    let vault_state = VaultState::new();
    // Invariant: `VaultState` serialization should never fail.
    VAULT_STATE.save(deps.storage, &vault_state).unwrap();

    let token_info = TokenInfo {
        name: msg.vault_info.vault_name,
        symbol: msg.vault_info.vault_symbol,
        decimals: 18,
        total_supply: Uint128::zero(),
        mint: Some(MinterData {
            minter: env.contract.address,
            cap: None,
        }),
    };
    // Invariant: `TokenInfo` serialization should never fail.
    TOKEN_INFO.save(deps.storage, &token_info).unwrap();

    Ok(Response::new())
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    use QueryMsg::*;
    match msg {
        VaultBalances {} => Ok(to_json_binary(&query::vault_balances(deps, &env))?),
        PositionBalancesWithFees { position_type } => Ok(to_json_binary(
            &query::position_balances_with_fees(position_type, deps),
        )?),
        CalcSharesAndUsableAmounts {
            for_amount0,
            for_amount1,
        } => Ok(to_json_binary(&query::calc_shares_and_usable_amounts(
            for_amount0,
            for_amount1,
            false,
            deps,
            &env,
        ))?),
        Balance { address } => to_json_binary(&query_balance(deps, address)?),
        VaultPositions {} => {
            // Invariant: Any state is present after instantiation.
            Ok(to_json_binary(&VAULT_STATE.load(deps.storage)?)?)
        }
    }
}
mod query {
    use std::str::FromStr;

    use osmosis_std::types::{
        cosmos::bank::v1beta1::BankQuerier,
        osmosis::concentratedliquidity::v1beta1::PositionByIdRequest,
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
        let (denom0, denom1) = VAULT_INFO
            .load(deps.storage)
            .unwrap()
            .pool_id
            .denoms(&deps.querier);

        let balances = BankQuerier::new(&deps.querier);
        let contract_addr = env.contract.address.to_string();

        // Invariant: Wont return `None` becuase we verify the pool and
        //            denoms are proper during instantiation.
        let contract_balance0 = balances
            .balance(contract_addr.clone(), denom0.clone())
            .ok()
            .unwrap()
            .balance
            .unwrap()
            .amount;

        let contract_balance1 = balances
            .balance(contract_addr, denom1.clone())
            .ok()
            .unwrap()
            .balance
            .unwrap()
            .amount;

        // Invariant: The conversion wont fail, because we got the
        //            contract balances directly from `BankQuerier.
        //            The additions wont overflow, because for that
        //            the token supply would have to be above
        //            `Uint128::MAX`, but thats not possible.
        let bal0 = Uint128::from_str(&contract_balance0)
            .unwrap()
            .checked_add(full_range_balances.bal0)
            .unwrap()
            .checked_add(base_balances.bal0)
            .unwrap()
            .checked_add(limit_balances.bal0)
            .unwrap();

        let bal1 = Uint128::from_str(&contract_balance1)
            .unwrap()
            .checked_add(full_range_balances.bal1)
            .unwrap()
            .checked_add(base_balances.bal1)
            .unwrap()
            .checked_add(limit_balances.bal1)
            .unwrap();

        VaultBalancesResponse { bal0, bal1 }
    }

    pub fn position_balances_with_fees(
        position_type: PositionType,
        deps: Deps,
    ) -> PositionBalancesWithFeesResponse {
        // Invariant: `VAULT_INFO` should always be present after instantiation.
        let (denom0, denom1) = VAULT_INFO
            .load(deps.storage as &dyn Storage)
            .unwrap()
            .pool_id
            .denoms(&deps.querier);
        // Invariant: `VAULT_STATE` should always be present after instantiation.
        let VaultState {
            full_range_position_id,
            base_position_id,
            limit_position_id,
        } = VAULT_STATE.load(deps.storage).unwrap();

        use PositionType::*;
        let id = match position_type {
            FullRange => full_range_position_id,
            Base => base_position_id,
            Limit => limit_position_id,
        };

        if id.is_none() {
            return PositionBalancesWithFeesResponse {
                bal0: Uint128::zero(),
                bal1: Uint128::zero(),
            };
        }
        let id = id.unwrap();

        // Invariant: We verified `id` is a valid position id the moment
        //            we put it in the state, so the query wont fail.
        let pos = PositionByIdRequest { position_id: id }
            .query(&deps.querier)
            .unwrap()
            .position
            .unwrap();

        // Invariant: If position is valid, both assets will be always present.
        let asset0 = pos.asset0.unwrap();
        let asset1 = pos.asset1.unwrap();
        let rewards = pos.claimable_spread_rewards;

        assert!(denom0 == asset0.denom && denom1 == asset1.denom);

        // Invariant: If `rewards` is present, we know its a `Vec` of valid
        //            amounts, so the conversion will never fail.
        let rewards0 = rewards
            .iter()
            .find(|x| x.denom == denom0)
            .map(|x| Uint128::from_str(&x.amount))
            .unwrap_or(Ok(Uint128::zero()))
            .unwrap();

        let rewards1 = rewards
            .iter()
            .find(|x| x.denom == denom1)
            .map(|x| Uint128::from_str(&x.amount))
            .unwrap_or(Ok(Uint128::zero()))
            .unwrap();

        // Invariant: Will never panic, because if the position has amounts
        //            `amount0` and `amount1`, we know theyre valid `Uint128`s.
        //            Neither will the addition overflow, because balances
        //            could only overflow if the tokens they refer had
        //            supplies above `Uint128::MAX`, but thats not possible.
        let bal0 = Uint128::from_str(&asset0.amount)
            .unwrap()
            .checked_add(rewards0)
            .unwrap();
        let bal1 = Uint128::from_str(&asset1.amount)
            .unwrap()
            .checked_add(rewards1)
            .unwrap();

        PositionBalancesWithFeesResponse { bal0, bal1 }
    }

    pub fn calc_shares_and_usable_amounts(
        input_amount0: Uint128,
        input_amount1: Uint128,
        amounts_already_in_contract: bool,
        deps: Deps,
        env: &Env,
    ) -> CalcSharesAndUsableAmountsResponse {
        let VaultBalancesResponse {
            bal0: total0,
            bal1: total1,
        } = query::vault_balances(deps, env);

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
                shares: (cmp::max(input_amount0, input_amount1)),
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
            let shares = Uint256::from(input_amount1)
                .checked_mul(total_supply.into())
                .unwrap()
                .checked_div(total1.into())
                .unwrap()
                .try_into()
                .unwrap();

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

            let shares = Uint256::from(input_amount0)
                .checked_mul(total_supply.into())
                .unwrap()
                .checked_div(total0.into())
                .unwrap()
                .try_into()
                .unwrap();

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

            let cross = cmp::min(
                input_amount0.checked_mul(total1).unwrap(),
                input_amount1.checked_mul(total0).unwrap(),
            );

            if cross.is_zero() {
                return CalcSharesAndUsableAmountsResponse {
                    shares: Uint128::zero(),
                    usable_amount0: Uint128::zero(),
                    usable_amount1: Uint128::zero(),
                };
            }

            let usable_amount0 = cross
                .checked_sub(Uint256::one())
                .unwrap()
                .checked_div(total1)
                .unwrap()
                .checked_add(Uint256::one())
                .unwrap()
                .try_into()
                .unwrap();

            let usable_amount1 = cross
                .checked_sub(Uint256::one())
                .unwrap()
                .checked_div(total0)
                .unwrap()
                .checked_add(Uint256::one())
                .unwrap()
                .try_into()
                .unwrap();

            let shares = cross
                .checked_mul(total_supply.into())
                .unwrap()
                .checked_div(total0)
                .unwrap()
                .checked_div(total1)
                .unwrap()
                .try_into()
                .unwrap();

            CalcSharesAndUsableAmountsResponse {
                shares,
                usable_amount0,
                usable_amount1,
            }
        }
    }
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    use ExecuteMsg::*;
    match msg {
        Deposit(deposit_msg) => exec::deposit(deposit_msg, deps, env, info),
        Rebalance {} => exec::rebalance(deps.as_ref(), env),
    }
}

mod exec {

    use crate::error::ContractError;
    use crate::msg::{CalcSharesAndUsableAmountsResponse, DepositMsg};
    use crate::state::{BLACKLISTED_ADDRESSES, TOKEN_INFO, VAULT_INFO};
    use cosmwasm_std::{BankMsg, Coin, Decimal, Uint128, Uint256};
    use cw_utils::must_pay;

    pub fn deposit(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: DepositMsg,
    ) -> Result<Response, ContractError> {
        let DepositMsg {
            amount0,
            amount1,
            amount0_min,
            amount1_min,
            to,
        } = msg;

        // Load vault info
        let vault_info = VAULT_INFO.load(deps.storage)?;
        let denom0 = vault_info.demon0(&deps.querier);
        let denom1 = vault_info.demon1(&deps.querier);

        // Validate and extract sent funds
        let amount0_sent = must_pay(&info, &denom0)?;
        let amount1_sent = must_pay(&info, &denom1)?;

        // Check if sent amounts match the requested amounts
        if amount0_sent != amount0 || amount1_sent != amount1 {
            return Err(ContractError::ImproperSentAmounts {
                expected: format!("({}, {})", amount0, amount1),
                got: format!("({}, {})", amount0_sent, amount1_sent),
            });
        }

        // Validate recipient address
        let recipient = deps.api.addr_validate(&to)?;

        // Check if recipient is blacklisted
        if BLACKLISTED_ADDRESSES
            .may_load(deps.storage, recipient.clone())?
            .is_some()
        {
            return Err(ContractError::BlacklistedAddress(recipient.to_string()));
        }

        // Check if recipient is a contract
        if deps
            .querier
            .query_wasm_smart_contract_info(recipient.clone())
            .is_ok()
        {
            return Err(ContractError::ContractAddressNotAllowed(
                recipient.to_string(),
            ));
        }

        // Prevent sending to the contract itself
        if recipient == env.contract.address {
            return Err(ContractError::ImproperSharesOwner(recipient.to_string()));
        }

        // Calculate shares and usable amounts
        let CalcSharesAndUsableAmountsResponse {
            shares,
            usable_amount0,
            usable_amount1,
        } = query::calc_shares_and_usable_amounts(amount0, amount1, true, deps.as_ref(), &env)?;

        // Check for minimum amounts
        if usable_amount0 < amount0_min || usable_amount1 < amount1_min {
            return Err(ContractError::DepositedAmountsBelowMin {
                used: format!("({}, {})", usable_amount0, usable_amount1),
                wanted: format!("({}, {})", amount0_min, amount1_min),
            });
        }

        // Mint shares
        execute_mint(
            deps,
            env.clone(),
            info.clone(),
            recipient.to_string(),
            shares,
        )?;

        // Calculate refund amounts
        let refund0 = amount0.checked_sub(usable_amount0)?;
        let refund1 = amount1.checked_sub(usable_amount1)?;

        // Prepare response
        let mut response = Response::new()
            .add_attribute("action", "deposit")
            .add_attribute("to", recipient.to_string())
            .add_attribute("shares", shares.to_string())
            .add_attribute("amount0_deposited", usable_amount0.to_string())
            .add_attribute("amount1_deposited", usable_amount1.to_string());

        // Refund unused amounts
        if !refund0.is_zero() {
            response = response.add_message(BankMsg::Send {
                to_address: info.sender.to_string(),
                amount: vec![Coin::new(refund0.u128(), denom0)],
            });
        }
        if !refund1.is_zero() {
            response = response.add_message(BankMsg::Send {
                to_address: info.sender.to_string(),
                amount: vec![Coin::new(refund1.u128(), denom1)],
            });
        }

        Ok(response)
    }

    fn execute_mint(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        recipient: String,
        amount: Uint128,
    ) -> Result<(), ContractError> {
        // Load token info
        let mut token_info = TOKEN_INFO.load(deps.storage)?;

        // Check minting permissions
        if token_info
            .mint
            .as_ref()
            .map_or(false, |m| m.minter != env.contract.address)
        {
            return Err(ContractError::Unauthorized {});
        }

        // Update total supply
        token_info.total_supply = token_info.total_supply.checked_add(amount)?;
        TOKEN_INFO.save(deps.storage, &token_info)?;

        // Mint tokens to recipient
        let rcpt_addr = deps.api.addr_validate(&recipient)?;
        cw20_base::contract::execute_mint(deps, env, info, rcpt_addr.to_string(), amount)?;

        Ok(())
    }

    pub fn remove_liquidity_msg(
        for_position: PositionType,
        deps: Deps,
        env: &Env,
    ) -> Option<MsgWithdrawPosition> {
        // Invariant: After instantiation, `VAULT_STATE` is always present.
        let VaultState {
            full_range_position_id,
            base_position_id,
            limit_position_id,
        } = VAULT_STATE.load(deps.storage).unwrap();

        use PositionType::*;
        let position_id = match for_position {
            FullRange => full_range_position_id,
            Base => base_position_id,
            Limit => limit_position_id,
        }?;

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

        let position_liquidity = Decimal::from_str(&position_liquidity)
            .unwrap()
            .atomics()
            .to_string();

        // TODO Also have to claim spread factor manually.
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
        let pool_id = &vault_info.pool_id;
        let pool = pool_id.to_pool(&deps.querier);

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
            token_min_amount1: raw(&slippage.mul_dec(&tokens_provided1)),
        }
    }

    // TODO Add attributes to Response to emit an event with info about the new positions.
    // TODO Finish cleanup.
    pub fn rebalance(deps: Deps, env: Env) -> Result<Response, ContractError> {
        // TODO Can rebalance? Check `VaultRebalancer` and other params,
        // like `minTickMove` or `period`.

        // Invariant: Any state will be initialized after instantation.
        let pool_id = &VAULT_INFO.load(deps.storage).unwrap().pool_id;
        let VaultParameters {
            base_factor,
            limit_factor,
            full_range_weight,
        } = VAULT_PARAMETERS.load(deps.storage).unwrap();

        let VaultBalancesResponse { bal0, bal1 } = query::vault_balances(deps, &env);
        // NOTE: We remove 3 tokens from each balance to prevent dust errors. Ie,
        //       position withdrawals always leave 1 token behind.
        let bal0 = bal0.checked_sub(Uint128::new(3)).unwrap_or(Uint128::zero());
        let bal1 = bal1.checked_sub(Uint128::new(3)).unwrap_or(Uint128::zero());

        if bal0.is_zero() && bal1.is_zero() {
            return Err(ContractError::NothingToRebalance {});
        }

        let price = pool_id.price(&deps.querier); // TODO: What if `price == 0`?

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
            let x0 = numerator
                .and_then(|n| n.checked_div(denominator).ok())
                .unwrap();
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

            // We take 1% slippage to check if balances have the right proportion.
            let balances_price = full_range_balance1 / full_range_balance0;
            assert!(balances_price >= price * Decimal::from_str("0.99").unwrap());
            assert!(balances_price <= price * Decimal::from_str("1.01").unwrap())
        }

        let (base_range_balance0, base_range_balance1) = if !base_factor.is_one() {
            // TODO Prove that those unwraps will never fail.
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

        let mut res: Response = Response::new();
        let mut new_position_msgs: Vec<SubMsg> = vec![];

        // If `full_range_balance0` is not zero, we already checked that neither
        // `full_range_balance1` will be. If they happened to be zero, it means that
        // the vault only holds tokens for limit orders for now, or that
        // the vault simply has zero `full_range_weight`.
        if !full_range_weight.is_zero() && !full_range_balance0.is_zero() {
            let lower_tick = pool_id.min_valid_tick(&deps.querier);
            let upper_tick = pool_id.max_valid_tick(&deps.querier);

            res = res
                .add_attribute("action", "create_full_range_position")
                .add_attribute("lower_tick", lower_tick.to_string())
                .add_attribute("upper_tick", upper_tick.to_string())
                .add_attribute("amount0", full_range_balance0.to_string())
                .add_attribute("amount1", full_range_balance1.to_string());

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
            let current_price = pool_id.price(&deps.querier);
            let lower_price = current_price
                .checked_div(base_factor.0)
                .unwrap_or(Decimal::MIN);

            let upper_price = current_price
                .checked_mul(base_factor.0)
                .unwrap_or(Decimal::MAX);

            let lower_tick = price_function_inv(&lower_price);
            let upper_tick = price_function_inv(&upper_price);

            res = res
                .add_attribute("action", "create_base_position")
                .add_attribute("lower_tick", lower_tick.to_string())
                .add_attribute("upper_tick", upper_tick.to_string())
                .add_attribute("amount0", base_range_balance0.to_string())
                .add_attribute("amount1", base_range_balance1.to_string());

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
            let current_price = pool_id.price(&deps.querier);
            if limit_balance0.is_zero() {
                let lower_price = current_price
                    .checked_div(limit_factor.0)
                    .unwrap_or(Decimal::MIN);

                let lower_tick = price_function_inv(&lower_price);

                // Invariant: Ticks nor Ticks spacings will never be large enough to
                //            overflow out of `i32`.
                let upper_tick = pool_id
                    .current_tick(&deps.querier)
                    .checked_sub(pool_id.tick_spacing(&deps.querier))
                    .unwrap();

                res = res
                    .add_attribute("action", "create_limit_position")
                    .add_attribute("lower_tick", lower_tick.to_string())
                    .add_attribute("upper_tick", upper_tick.to_string())
                    .add_attribute("amount0", limit_balance0.to_string())
                    .add_attribute("amount1", limit_balance1.to_string());

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
                let upper_price = current_price
                    .checked_mul(limit_factor.0)
                    .unwrap_or(Decimal::MIN);

                let upper_tick = price_function_inv(&upper_price);

                // Invariant: Ticks nor Ticks spacings will never be large enough to
                //            overflow out of `i32`.
                let lower_tick = pool_id
                    .current_tick(&deps.querier)
                    .checked_add(pool_id.tick_spacing(&deps.querier))
                    .unwrap();

                res = res
                    .add_attribute("action", "create_limit_position")
                    .add_attribute("lower_tick", lower_tick.to_string())
                    .add_attribute("upper_tick", upper_tick.to_string())
                    .add_attribute("amount0", limit_balance0.to_string())
                    .add_attribute("amount1", limit_balance1.to_string());

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

        let mut liquidity_removal_msgs: Vec<MsgWithdrawPosition> = vec![];

        if let Some(msg) = remove_liquidity_msg(PositionType::FullRange, deps, &env) {
            liquidity_removal_msgs.push(msg)
        }
        if let Some(msg) = remove_liquidity_msg(PositionType::Base, deps, &env) {
            liquidity_removal_msgs.push(msg)
        }
        if let Some(msg) = remove_liquidity_msg(PositionType::Limit, deps, &env) {
            liquidity_removal_msgs.push(msg)
        }

        // TODO Add callback for protocol fees and manager fees.
        let position_ids = liquidity_removal_msgs
            .iter()
            .map(|msg| msg.position_id)
            .collect();

        let rewards_claim_msg = MsgCollectSpreadRewards {
            position_ids,
            sender: env.contract.address.into(),
        };

        Ok(res
            .add_message(rewards_claim_msg)
            .add_messages(liquidity_removal_msgs)
            .add_submessages(new_position_msgs))
    }
}

// TODO: Prove all unwraps security.
#[entry_point]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    let new_position: MsgCreatePositionResponse = msg.result.try_into().unwrap();
    let mut vault_state = VAULT_STATE.load(deps.storage).unwrap();

    match msg.id {
        0 => vault_state.full_range_position_id = Some(new_position.position_id),
        1 => vault_state.base_position_id = Some(new_position.position_id),
        2 => vault_state.limit_position_id = Some(new_position.position_id),
        _ => unreachable!(),
    };

    VAULT_STATE.save(deps.storage, &vault_state).unwrap();

    Ok(Response::new())
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use crate::{
        constants::{MAX_TICK, MIN_TICK},
        msg::{
            DepositMsg, VaultInfoInstantiateMsg, VaultParametersInstantiateMsg,
            VaultRebalancerInstantiateMsg,
        },
        state::price_function_inv,
    };

    use super::*;
    use cosmwasm_std::{Coin, Decimal};
    use osmosis_std::types::osmosis::{
        concentratedliquidity::v1beta1::{
            CreateConcentratedLiquidityPoolsProposal, MsgCreatePosition, PoolRecord,
        },
        poolmanager::v1beta1::{MsgSwapExactAmountIn, SwapAmountInRoute},
    };
    use osmosis_test_tube::{
        Account, ConcentratedLiquidity, GovWithAppAccess, Module, OsmosisTestApp, PoolManager,
        SigningAccount, Wasm,
    };

    struct PoolMockupInfo {
        pool_id: u64,
        app: OsmosisTestApp,
        deployer: SigningAccount,
        price: Decimal,
    }

    const USDC_DENOM: &str = "ibc/DE6792CF9E521F6AD6E9A4BDF6225C9571A3B74ACC0A529F92BC5122A39D2E58";
    const OSMO_DENOM: &str = "uosmo";

    fn create_basic_usdc_osmo_pool(x_bal: u128, y_bal: u128) -> PoolMockupInfo {
        let app = OsmosisTestApp::new();
        let deployer = app
            .init_account(&[
                Coin::new(1_000_000_000_000u128, USDC_DENOM),
                Coin::new(1_000_000_000_000u128, OSMO_DENOM),
            ])
            .unwrap();

        let cl = ConcentratedLiquidity::new(&app);
        let gov = GovWithAppAccess::new(&app);

        // Pool setup.
        gov.propose_and_execute(
            CreateConcentratedLiquidityPoolsProposal::TYPE_URL.to_string(),
            CreateConcentratedLiquidityPoolsProposal {
                title: "Create cl uosmo:usdc pool".into(),
                description: "blabla".into(),
                pool_records: vec![PoolRecord {
                    denom0: USDC_DENOM.into(),
                    denom1: OSMO_DENOM.into(),
                    tick_spacing: 100,
                    spread_factor: "0".into(),
                }],
            },
            deployer.address(),
            &deployer,
        )
        .unwrap();
        let pool_id = 1;

        let position_res = cl
            .create_position(
                MsgCreatePosition {
                    pool_id,
                    sender: deployer.address(),
                    lower_tick: MIN_TICK.into(),
                    upper_tick: MAX_TICK.into(),
                    tokens_provided: vec![
                        Coin::new(x_bal, USDC_DENOM).into(),
                        Coin::new(y_bal, OSMO_DENOM).into(),
                    ],
                    token_min_amount0: x_bal.to_string(),
                    token_min_amount1: y_bal.to_string(),
                },
                &deployer,
            )
            .unwrap()
            .data;

        assert_eq!(position_res.position_id, 1);

        PoolMockupInfo {
            pool_id,
            app,
            deployer,
            price: Decimal::new(y_bal.into()) / Decimal::new(x_bal.into()),
        }
    }

    fn store_vaults_code(wasm: &Wasm<OsmosisTestApp>, deployer: &SigningAccount) -> u64 {
        let contract_bytecode =
            std::fs::read("target/wasm32-unknown-unknown/release/magma_core.wasm").unwrap();

        wasm.store_code(&contract_bytecode, None, deployer)
            .unwrap()
            .data
            .code_id
    }

    #[readonly::make]
    pub struct VaultAddr(pub String);
    fn inst_vault(
        pool_info: &PoolMockupInfo,
        params: VaultParametersInstantiateMsg,
    ) -> (VaultAddr, Wasm<OsmosisTestApp>) {
        let wasm = Wasm::new(&pool_info.app);
        let code_id = store_vaults_code(&wasm, &pool_info.deployer);

        let vault_addr = wasm
            .instantiate(
                code_id,
                &InstantiateMsg {
                    vault_info: VaultInfoInstantiateMsg {
                        pool_id: pool_info.pool_id,
                        vault_name: "My USDC/OSMO vault".into(),
                        vault_symbol: "USDCOSMOV".into(),
                        admin: Some(pool_info.deployer.address()),
                        rebalancer: VaultRebalancerInstantiateMsg::Admin {},
                    },
                    vault_parameters: params,
                },
                None,
                Some("my vault"),
                &[],
                &pool_info.deployer,
            )
            .unwrap()
            .data
            .address;

        (VaultAddr(vault_addr), wasm)
    }

    #[test]
    fn price_function_inv_test() {
        let prices = &[
            Decimal::from_str("0.099998").unwrap(),
            Decimal::from_str("0.099999").unwrap(),
            Decimal::from_str("0.94998").unwrap(),
            Decimal::from_str("0.94999").unwrap(),
            Decimal::from_str("0.99998").unwrap(),
            Decimal::from_str("0.99999").unwrap(),
            Decimal::from_str("1").unwrap(),
            Decimal::from_str("1.0001").unwrap(),
            Decimal::from_str("1.0002").unwrap(),
            Decimal::from_str("9.9999").unwrap(),
            Decimal::from_str("10.001").unwrap(),
            Decimal::from_str("10.002").unwrap(),
        ];

        let ticks = &[
            -9000200, -9000100, -500200, -500100, -200, -100, 0, 100, 200, 8999900, 9000100,
            9000200,
        ];

        for (p, expected_tick) in prices.iter().zip(ticks.iter()) {
            let got_tick = price_function_inv(p);
            assert_eq!(*expected_tick, got_tick)
        }
    }

    #[test]
    fn normal_rebalance() {
        let pool_info = create_basic_usdc_osmo_pool(100_000, 200_000);
        let (vault_addr, wasm) = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: "2".into(),
                limit_factor: "1.45".into(),
                full_range_weight: "0.55".into(),
            },
        );

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(1_000),
                amount1: Uint128::new(1_500),
                amount0_min: Uint128::new(1_000),
                amount1_min: Uint128::new(1_500),
                to: pool_info.deployer.address(),
            }),
            &[
                Coin::new(1_000, USDC_DENOM).into(),
                Coin::new(1_500, OSMO_DENOM).into(),
            ],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();
    }

    #[test]
    fn normal_rebalance_dual() {
        let pool_info = create_basic_usdc_osmo_pool(100_000, 200_000);
        let (vault_addr, wasm) = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: "2".into(),
                limit_factor: "1.45".into(),
                full_range_weight: "0.55".into(),
            },
        );

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(1_500),
                amount1: Uint128::new(1_000),
                amount0_min: Uint128::new(1_500),
                amount1_min: Uint128::new(1_000),
                to: pool_info.deployer.address(),
            }),
            &[
                Coin::new(1_500, USDC_DENOM).into(),
                Coin::new(1_000, OSMO_DENOM).into(),
            ],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();
    }

    #[test]
    fn rebalance_in_proportion() {
        let pool_balance0 = 100_000;
        let pool_balance1 = 200_000;
        let pool_info = create_basic_usdc_osmo_pool(pool_balance0, pool_balance1);

        let (vault_addr, wasm) = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: "2".into(),
                limit_factor: "1.45".into(),
                full_range_weight: "0.55".into(),
            },
        );

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(pool_balance0 / 2),
                amount1: Uint128::new(pool_balance1 / 2),
                amount0_min: Uint128::new(pool_balance0 / 2),
                amount1_min: Uint128::new(pool_balance1 / 2),
                to: pool_info.deployer.address(),
            }),
            &[
                Coin::new(pool_balance0 / 2, USDC_DENOM).into(),
                Coin::new(pool_balance1 / 2, OSMO_DENOM).into(),
            ],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();
    }

    #[test]
    fn only_limit_rebalance() {
        let pool_info = create_basic_usdc_osmo_pool(100_000, 200_000);
        let (vault_addr, wasm) = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: "2".into(),
                limit_factor: "1.45".into(),
                full_range_weight: "0.55".into(),
            },
        );

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(42),
                amount1: Uint128::new(0),
                amount0_min: Uint128::new(42),
                amount1_min: Uint128::new(0),
                to: pool_info.deployer.address(),
            }),
            &[Coin::new(42, USDC_DENOM).into()],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();
    }

    #[test]
    fn only_limit_rebalance_dual() {
        let pool_info = create_basic_usdc_osmo_pool(100_000, 200_000);
        let (vault_addr, wasm) = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: "2".into(),
                limit_factor: "1.45".into(),
                full_range_weight: "0.55".into(),
            },
        );

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(0),
                amount1: Uint128::new(42),
                amount0_min: Uint128::new(0),
                amount1_min: Uint128::new(42),
                to: pool_info.deployer.address(),
            }),
            &[Coin::new(42, OSMO_DENOM).into()],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();
    }

    #[test]
    fn vault_positions() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_info = create_basic_usdc_osmo_pool(pool_x, pool_y);
        let base_factor = Decimal::from_str("2").unwrap();
        let limit_factor = Decimal::from_str("1.45").unwrap();
        let full_range_weight = Decimal::from_str("0.55").unwrap();

        let (vault_addr, wasm) = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: base_factor.to_string(),
                limit_factor: limit_factor.to_string(),
                full_range_weight: full_range_weight.to_string(),
            },
        );

        let (vault_x, vault_y) = (1_000, 1_000);
        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
                amount0_min: Uint128::new(vault_x),
                amount1_min: Uint128::new(vault_y),
                to: pool_info.deployer.address(),
            }),
            &[
                Coin::new(vault_x, USDC_DENOM).into(),
                Coin::new(vault_y, OSMO_DENOM).into(),
            ],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();

        let pm = PoolManager::new(&pool_info.app);
        let usdc_got = pm
            .swap_exact_amount_in(
                MsgSwapExactAmountIn {
                    sender: pool_info.deployer.address(),
                    routes: vec![SwapAmountInRoute {
                        pool_id: pool_info.pool_id,
                        token_out_denom: USDC_DENOM.into(),
                    }],
                    token_in: Some(Coin::new(pool_y / 10, OSMO_DENOM).into()),
                    token_out_min_amount: "1".into(),
                },
                &pool_info.deployer,
            )
            .unwrap()
            .data
            .token_out_amount;
        let usdc_got = Uint128::from_str(&usdc_got).unwrap();

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();

        pm.swap_exact_amount_in(
            MsgSwapExactAmountIn {
                sender: pool_info.deployer.address(),
                routes: vec![SwapAmountInRoute {
                    pool_id: pool_info.pool_id,
                    token_out_denom: OSMO_DENOM.into(),
                }],
                token_in: Some(Coin::new(usdc_got.into(), USDC_DENOM).into()),
                token_out_min_amount: "1".into(),
            },
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.0,
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();
    }
}
