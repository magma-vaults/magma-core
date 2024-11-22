use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Reply, Response,
    StdResult, Uint128,
};
use cw20_base::allowances::{
    execute_burn_from, execute_decrease_allowance, execute_increase_allowance, execute_send_from,
    execute_transfer_from, query_allowance,
};
use cw20_base::contract::{
    execute_burn, execute_send, execute_transfer, query_balance, query_token_info,
};
use cw20_base::state::{MinterData, TokenInfo, TOKEN_INFO};
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::MsgCreatePositionResponse;

use crate::msg::QueryMsg;
use crate::state::{FeesInfo, FundsInfo, FEES_INFO, FUNDS_INFO};
use crate::{do_me, execute, query};
use crate::{
    error::ContractError,
    msg::{ExecuteMsg, InstantiateMsg},
    state::{VaultInfo, VaultParameters, VaultState, VAULT_INFO, VAULT_PARAMETERS, VAULT_STATE},
};

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {

    let vault_info = VaultInfo::new(msg.vault_info.clone(), deps.as_ref())?;
    let vault_parameters = VaultParameters::new(msg.vault_parameters.clone())?;
    let vault_state = VaultState::default();
    let fees_info = FeesInfo::new(msg.vault_info.admin_fee, &vault_info, &info)?;
    let funds_info = FundsInfo::default();
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

    // Invariant: No state serializaton will panic, as we already ensured
    //            the types are proper during development.
    do_me! {
        VAULT_INFO.save(deps.storage, &vault_info)?;
        VAULT_PARAMETERS.save(deps.storage, &vault_parameters)?;
        VAULT_STATE.save(deps.storage, &vault_state)?;
        FEES_INFO.save(deps.storage, &fees_info)?;
        FUNDS_INFO.save(deps.storage, &funds_info)?;
        TOKEN_INFO.save(deps.storage, &token_info)?;
    }.unwrap();

    Ok(Response::new())
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    use QueryMsg::*;
    match msg {
        PositionBalancesWithFees { position_type } => 
            to_json_binary( &query::position_balances_with_fees(position_type, deps),),
        CalcSharesAndUsableAmounts { for_amount0, for_amount1, } => 
            to_json_binary(&query::calc_shares_and_usable_amounts(for_amount0, for_amount1, deps)),
        VaultBalances {} => to_json_binary(&query::vault_balances(deps)),
        Balance { address } => to_json_binary(&query_balance(deps, address)?),
        Allowance { owner, spender } => to_json_binary(&query_allowance(deps, owner, spender)?),
        // Invariant: Any state is present after instantiation.
        VaultState {} => to_json_binary(&VAULT_STATE.load(deps.storage).unwrap()),
        VaultParameters {} => to_json_binary(&VAULT_PARAMETERS.load(deps.storage).unwrap()),
        VaultInfo {} => to_json_binary(&VAULT_INFO.load(deps.storage).unwrap()),
        FeesInfo {} => to_json_binary(&FEES_INFO.load(deps.storage).unwrap()),
        TokenInfo {} => to_json_binary(&query_token_info(deps)?),
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

    if !matches!(msg, Deposit(_)) && !info.funds.is_empty() {
        return Err(ContractError::NonPayable(format!("{:?}", msg)))
    }

    match msg {
        // Core Logic.
        Deposit(deposit_msg) => Ok(execute::deposit(deposit_msg, deps, env, info)?),
        Rebalance {} => Ok(execute::rebalance(deps, env, info)?),
        Withdraw(withdraw_msg) => Ok(execute::withdraw(withdraw_msg, deps, env, info)?),

        // Admin/Protocol operations.
        WithdrawProtocolFees {} => Ok(execute::withdraw_protocol_fees(deps, info)?),
        WithdrawAdminFees {} => Ok(execute::withdraw_admin_fees(deps, info)?),
        ProposeNewAdmin { new_admin } => Ok(execute::propose_new_admin(deps, info, new_admin)?),
        AcceptNewAdmin {} => Ok(execute::accept_new_admin(deps, info)?),
        BurnVaultAdmin {} => Ok(execute::burn_vault_admin(deps, info)?),
        ChangeVaultRebalancer(rebalancer) => Ok(execute::change_vault_rebalancer(rebalancer, deps, info)?),
        ChangeVaultParameters(parameters) => Ok(execute::change_vault_parameters(parameters, deps, info)?),
        ChangeAdminFee { new_admin_fee } => Ok(execute::change_admin_fee(new_admin_fee, deps, info)?),
        ChangeProtocolFee { new_protocol_fee } => Ok(execute::change_protocol_fee(new_protocol_fee, deps, info)?),

        // Cw20 Realization.
        Transfer { recipient, amount } => Ok(execute_transfer(deps, env, info, recipient, amount)?),
        Burn { amount } => Ok(execute_burn(deps, env, info, amount)?),
        Send { contract, amount, msg } => Ok(execute_send(deps, env, info, contract, amount, msg)?),
        IncreaseAllowance { spender, amount, expires } => Ok(execute_increase_allowance( deps, env, info, spender, amount, expires)?),
        DecreaseAllowance { spender, amount, expires } => Ok(execute_decrease_allowance( deps, env, info, spender, amount, expires)?),
        TransferFrom { owner, recipient, amount, } => Ok(execute_transfer_from(deps, env, info, owner, recipient, amount)?),
        BurnFrom { owner, amount } => Ok(execute_burn_from(deps, env, info, owner, amount)?),
        SendFrom { owner, contract, amount, msg, } => Ok(execute_send_from( deps, env, info, owner, contract, amount, msg)?),
    }
}

#[entry_point]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    // Invariant: We only use position creation submessages.
    let new_position: MsgCreatePositionResponse = msg.result.try_into().unwrap();
    // Invariant: Any state will always be present after instantiation.
    let mut vault_state = VAULT_STATE.load(deps.storage).unwrap();

    match msg.id {
        0 => vault_state.full_range_position_id = Some(new_position.position_id),
        1 => vault_state.base_position_id = Some(new_position.position_id),
        2 => vault_state.limit_position_id = Some(new_position.position_id),
        _ => unreachable!(),
    };

    // Invariant: Wont panic as all types are proper.
    VAULT_STATE.save(deps.storage, &vault_state).unwrap();

    Ok(Response::new())
}

#[cfg(test)]
pub mod test {

    use std::str::FromStr;

    use crate::{assert_approx_eq, constants::MIN_LIQUIDITY, mock::mock::{deposit_msg, rebalancer_anyone, vault_params, PoolMockup, VaultMockup, OSMO_DENOM, USDC_DENOM}, msg::{DepositMsg, WithdrawMsg}, state::PositionType, utils::price_function_inv};

    use super::*;
    use cosmwasm_std::{coin, Coin, Decimal};
    use osmosis_test_tube::{Account, ConcentratedLiquidity, Module, OsmosisTestApp};

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
            -9000200, -9000100, -500200, -500100, -200, -100, 0, 100, 200, 8999900, 9000100, 9000200
        ];

        for (p, expected_tick) in prices.iter().zip(ticks.iter()) {
            let got_tick = price_function_inv(p);
            assert_eq!(*expected_tick, got_tick)
        }
    }

    #[test]
    fn normal_rebalances() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup.deposit(1_000, 1_501, &pool_mockup.user1).unwrap();
        let bals = vault_mockup.vault_balances_query();
        assert_eq!(bals.bal0.u128(), 1_000);
        assert_eq!(bals.bal1.u128(), 1_501);
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        let full_range_position = vault_mockup.position_balances_query(PositionType::FullRange);

        // \[
        //   x_0 = \frac{\sqrt k X w}{\sqrt k - 1 + w} 
        //       = \frac{\sqrt 2 \cdot 750 \cdot 0.55}{\sqrt 2 - 1 + 0.55 }
        //       \approx 605$
        // \]
        assert_approx_eq!(full_range_position.bal0, Uint128::new(605), Uint128::new(5));
        // \[ y_0 = x_0 p \]
        assert_approx_eq!(full_range_position.bal1, Uint128::new(605 * 2), Uint128::new(5));
    }

    #[test]
    fn normal_rebalance_dual() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup.deposit(1_000, 1_500, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
    }

    #[test]
    fn rebalance_in_proportion() {
        let pool_balance0 = 100_000;
        let pool_balance1 = 200_000;
        let pool_mockup = PoolMockup::new(pool_balance0, pool_balance1);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        
        vault_mockup.deposit(pool_balance0/2, pool_balance1/2, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        assert!(vault_mockup.vault_state_query().limit_position_id.is_none());
        assert!(vault_mockup.vault_state_query().full_range_position_id.is_some());
        assert!(vault_mockup.vault_state_query().base_position_id.is_some());
    }

    #[test]
    fn only_limit_rebalance() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup.deposit(10_123, 0, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        // Dual case
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup.deposit(0, 10_123, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        // Combined case
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup.deposit(10_123, 0, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        assert!(vault_mockup.vault_state_query().limit_position_id.is_some());
        assert!(vault_mockup.vault_state_query().full_range_position_id.is_none());
        assert!(vault_mockup.vault_state_query().base_position_id.is_none());

        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();
        // FIXME: See issue #1. (FIXME What was this again? issue #1 links to a PR.
        // assert!(vault_mockup.vault_state_query().limit_position_id.is_none());
        // assert!(vault_mockup.vault_state_query().full_range_position_id.is_none());
        // assert!(vault_mockup.vault_state_query().base_position_id.is_none());
        // vault_mockup.deposit(0, 42, &pool_mockup.user1).unwrap();
        // vault_mockup.rebalance(&pool_mockup.user1).unwrap();
        // assert!(vault_mockup.vault_state_query().limit_position_id.is_some());
        // assert!(vault_mockup.vault_state_query().full_range_position_id.is_none());
        // assert!(vault_mockup.vault_state_query().base_position_id.is_none());
    }

    #[test]
    fn full_limit_liquidation() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        
        vault_mockup.deposit(50_000, 0, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();

        vault_mockup.deposit(50_000, 0, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();
    }

    #[test]
    fn full_balanced_liquidation() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        
        vault_mockup.deposit(10_000, 20_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();

        vault_mockup.deposit(10_000, 20_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();
    }

    #[test]
    fn full_liquidation() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        
        vault_mockup.deposit(10_000, 25_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();

        vault_mockup.deposit(10_000, 25_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();
    }

    #[test]
    fn rebalance_after_price_change() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (10_000, 10_000);
        vault_mockup.deposit(vault_x, vault_y, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        let usdc_got = pool_mockup.swap_osmo_for_usdc(&pool_mockup.user1, vault_y/10).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        pool_mockup.swap_usdc_for_osmo(&pool_mockup.user1, usdc_got.into()).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
    }

    #[test]
    fn out_of_range_vault_positions_test() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (10_000, 15_000);
        vault_mockup.deposit(vault_x, vault_y, &pool_mockup.user1).unwrap();
        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        pool_mockup.swap_usdc_for_osmo(&pool_mockup.user1, 50_000).unwrap();
        vault_mockup.withdraw(shares_got, &pool_mockup.user1).unwrap();
    }


    #[test]
    fn withdraw_without_rebalances() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_mockup = PoolMockup::new(pool_x, pool_y);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (10_000, 15_000);
        vault_mockup.deposit(vault_x, vault_y, &pool_mockup.user1).unwrap();
        let vault_bals_before_withdrawal = vault_mockup.vault_balances_query();
        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        assert!(vault_mockup.withdraw(shares_got, &pool_mockup.user2).is_err());
        vault_mockup.withdraw(shares_got, &pool_mockup.user1).unwrap();
        let vault_bals_after_withdrawal = vault_mockup.vault_balances_query();

        assert_eq!(vault_bals_before_withdrawal.bal0, Uint128::new(vault_x));
        assert_eq!(vault_bals_before_withdrawal.bal1, Uint128::new(vault_y));
        assert_approx_eq!(vault_bals_after_withdrawal.bal0, Uint128::zero(), MIN_LIQUIDITY + Uint128::one());
        assert_approx_eq!(vault_bals_after_withdrawal.bal1, Uint128::zero(), MIN_LIQUIDITY + Uint128::one());
    }


    #[test]
    fn withdraw_limit_without_rebalances() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_mockup = PoolMockup::new(pool_x, pool_y);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (0, 6969);
        vault_mockup.deposit(vault_x, vault_y, &pool_mockup.user1).unwrap();
        
        assert!(vault_mockup.vault_balances_query().bal0.is_zero());
        assert_eq!(vault_mockup.vault_balances_query().bal1, Uint128::new(vault_y));

        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares_got, &pool_mockup.user1).unwrap();

        assert!(vault_mockup.vault_balances_query().bal0.is_zero());
        assert_approx_eq!(vault_mockup.vault_balances_query().bal1, Uint128::zero(), MIN_LIQUIDITY + Uint128::one());
    }

    #[test]
    fn withdraw_with_min_amounts() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_mockup = PoolMockup::new(pool_x, pool_y);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (10_000, 15_000);

        let improper_deposit = vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0_min: Uint128::new(vault_x) + Uint128::one(),
                amount1_min: Uint128::new(vault_y) + Uint128::one(),
                to: pool_mockup.user1.address()
            }),
            &[
                coin(vault_x, USDC_DENOM),
                coin(vault_y, OSMO_DENOM)
            ],
            &pool_mockup.user1
        );
        assert!(improper_deposit.is_err());

        vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0_min: Uint128::new(vault_x),
                amount1_min: Uint128::new(vault_y),
                to: pool_mockup.user1.address()
            }),
            &[
                coin(vault_x, USDC_DENOM),
                coin(vault_y, OSMO_DENOM)
            ],
            &pool_mockup.user1
        ).unwrap();


        let vault_balances_before_withdrawal = vault_mockup.vault_balances_query();
        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());

        let improper_withdrawal = vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(), 
            &ExecuteMsg::Withdraw(
                WithdrawMsg {
                    shares: shares_got,
                    amount0_min: vault_balances_before_withdrawal.bal0 - MIN_LIQUIDITY,
                    amount1_min: vault_balances_before_withdrawal.bal1 - MIN_LIQUIDITY,
                    to: pool_mockup.user1.address()
                }
            ),
            &[],
            &pool_mockup.user1
        );
        assert!(improper_withdrawal.is_err());

        vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(), 
            &ExecuteMsg::Withdraw(
                WithdrawMsg {
                    shares: shares_got,
                    amount0_min: vault_balances_before_withdrawal.bal0 - MIN_LIQUIDITY - Uint128::one(),
                    amount1_min: vault_balances_before_withdrawal.bal1 - MIN_LIQUIDITY - Uint128::one(),
                    to: pool_mockup.user1.address()
                }
            ),
            &[],
            &pool_mockup.user1
        ).unwrap();

        let vault_balances_after_withdrawal = vault_mockup.vault_balances_query();

        assert_approx_eq!(vault_balances_after_withdrawal.bal0, Uint128::zero(), MIN_LIQUIDITY + Uint128::one());
        assert_approx_eq!(vault_balances_after_withdrawal.bal1, Uint128::zero(), MIN_LIQUIDITY + Uint128::one());
        assert!(vault_balances_after_withdrawal.protocol_unclaimed_fees0.is_zero());
        assert!(vault_balances_after_withdrawal.protocol_unclaimed_fees1.is_zero());
    }

    #[test]
    fn fees_withdrawals_on_rebalance() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        vault_mockup.deposit(100_000, 50_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        pool_mockup.swap_osmo_for_usdc(&pool_mockup.user2, 20_000).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        let fees = vault_mockup.vault_fees_query();
        assert!(fees.admin_tokens0_owned.is_zero());
        assert!(!fees.admin_tokens1_owned.is_zero());
        assert!(fees.protocol_tokens0_owned.is_zero());
        assert!(!fees.protocol_tokens1_owned.is_zero());

        assert!(vault_mockup.admin_withdraw(&pool_mockup.user1).is_err());
        assert!(vault_mockup.admin_withdraw(&pool_mockup.user2).is_err());
        vault_mockup.admin_withdraw(&pool_mockup.deployer).unwrap();
        vault_mockup.admin_withdraw(&pool_mockup.deployer).unwrap();

        let fees = vault_mockup.vault_fees_query();
        assert!(fees.admin_tokens0_owned.is_zero());
        assert!(fees.admin_tokens1_owned.is_zero());
        assert!(fees.protocol_tokens0_owned.is_zero());
        assert!(!fees.protocol_tokens1_owned.is_zero());

        // TODO
        // vault_mockup.protocol_withdraw().unwrap();
    }

    #[test]
    fn fees_withdrawals_on_withdrawal() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        vault_mockup.deposit(100_000, 50_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());

        pool_mockup.swap_osmo_for_usdc(&pool_mockup.user2, 20_000).unwrap();
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();

        let fees = vault_mockup.vault_fees_query();
        assert!(fees.admin_tokens0_owned.is_zero());
        assert!(!fees.admin_tokens1_owned.is_zero());
        assert!(fees.protocol_tokens0_owned.is_zero());
        assert!(!fees.protocol_tokens1_owned.is_zero());

        assert!(vault_mockup.admin_withdraw(&pool_mockup.user1).is_err());
        assert!(vault_mockup.admin_withdraw(&pool_mockup.user2).is_err());
        let _x = vault_mockup.admin_withdraw(&pool_mockup.deployer).unwrap();
        let _y = vault_mockup.admin_withdraw(&pool_mockup.deployer).unwrap();
        // TODO Check if the transaction indeed sends some tokens back.

        let fees = vault_mockup.vault_fees_query();
        assert!(fees.admin_tokens0_owned.is_zero());
        assert!(fees.admin_tokens1_owned.is_zero());
        assert!(fees.protocol_tokens0_owned.is_zero());
        assert!(!fees.protocol_tokens1_owned.is_zero());

        // TODO
        // vault_mockup.protocol_withdraw().unwrap();
    }

    #[test] 
    fn cant_operate_with_no_funds() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        assert!(vault_mockup.rebalance(&pool_mockup.deployer).is_err());
        assert!(vault_mockup.withdraw(Decimal::one().atomics(), &pool_mockup.deployer).is_err());
        assert!(vault_mockup.withdraw(Uint128::zero(), &pool_mockup.deployer).is_err());
    }

    #[test]
    fn cant_manipulate_contract_balances_in_unintended_ways() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        vault_mockup.deposit(50_000, 50_000, &pool_mockup.user1).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());

        let should_err = vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &ExecuteMsg::Withdraw(WithdrawMsg { 
                shares, 
                amount0_min: Uint128::zero(),
                amount1_min: Uint128::zero(),
                to: pool_mockup.user1.address()
            }),
            &[coin(1000, USDC_DENOM)],
            &pool_mockup.user1
        );
        assert!(should_err.is_err());
    }

    #[test]
    fn min_liquidity_attack() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup.deposit(10_000, 10_000, &pool_mockup.user1).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares - Uint128::one(), &pool_mockup.user1).unwrap();

        vault_mockup.deposit(10_000, 10_000, &pool_mockup.user2).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user2.address());
        vault_mockup.withdraw(shares - Uint128::one(), &pool_mockup.user2).unwrap();
    }

    #[test]
    fn partial_withdrawal_without_rebalance() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let usdc_amount = 10_000;
        let osmo_amount = 10_000;
        vault_mockup.deposit(usdc_amount, osmo_amount, &pool_mockup.user1).unwrap();

        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        assert_eq!((shares + MIN_LIQUIDITY).u128(), usdc_amount);

        vault_mockup.withdraw(Uint128::new(4444), &pool_mockup.user1).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();

        let bals = vault_mockup.vault_balances_query();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
    }

    #[test]
    fn partial_withdrawal_with_rebalance() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let usdc_amount = 10_000;
        let osmo_amount = 10_000;
        vault_mockup.deposit(usdc_amount, osmo_amount, &pool_mockup.user1).unwrap();

        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        assert_eq!((shares + MIN_LIQUIDITY).u128(), usdc_amount);

        vault_mockup.withdraw(Uint128::new(4444), &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares/Uint128::new(2), &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        assert!(shares.is_zero());
    }
    
    #[test]
    fn public_first_rebalancing() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new_with_rebalancer(
            &pool_mockup,
            vault_params("2", "1.45", "0.55"),
            rebalancer_anyone("1", 69)
        );
        vault_mockup.deposit(10_000, 10_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user2).unwrap();
    }

    #[test]
    fn public_rebalancing_at_due_time() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let seconds_before_rebalance = 3600;
        let vault_mockup = VaultMockup::new_with_rebalancer(
            &pool_mockup,
            vault_params("2", "1.45", "0.55"),
            rebalancer_anyone("1", seconds_before_rebalance)
        );
        vault_mockup.deposit(10_000, 10_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user2).unwrap();

        // Hypothesis: `6` as each operation takes 3 seconds.
        pool_mockup.app.increase_time((seconds_before_rebalance - 6).into());
        assert!(vault_mockup.rebalance(&pool_mockup.user1).is_err());
        assert!(vault_mockup.rebalance(&pool_mockup.deployer).is_err());
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();
        assert!(vault_mockup.rebalance(&pool_mockup.deployer).is_err());
    }

    #[test]
    fn public_rebalancing_after_price_moved() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new_with_rebalancer(
            &pool_mockup,
            vault_params("2", "1.45", "0.55"),
            rebalancer_anyone("1.01", 0)
        );
        vault_mockup.deposit(10_000, 10_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user2).unwrap();
        pool_mockup.app.increase_time(1);
        assert!(vault_mockup.rebalance(&pool_mockup.user1).is_err());
        pool_mockup.swap_osmo_for_usdc(&pool_mockup.user2, 10_000).unwrap();

        // NOTE: We wait for the price to settle so we dont get an TWAP error.
        assert!(vault_mockup.rebalance(&pool_mockup.user1).is_err());
        pool_mockup.app.increase_time(30);
        assert!(vault_mockup.rebalance(&pool_mockup.user1).is_err());
        pool_mockup.app.increase_time(30);
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();
    }

    #[test]
    fn cant_deposit_improper_tokens() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        let improper_token = "ibc/000000000000000000000000000000000000000000000000000000000000DEAD";

        let improper_user = pool_mockup.app.init_account(&[
            Coin::new(1_000_000_000_000u128, USDC_DENOM),
            Coin::new(1_000_000_000_000u128, OSMO_DENOM),
            Coin::new(1_000_000_000_000u128, improper_token),
        ]).unwrap();

        assert!(vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &deposit_msg(improper_user.address()),
            &[Coin::new(10_000, USDC_DENOM), Coin::new(10_000, improper_token)],
            &improper_user
        ).is_err());

        assert!(vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &deposit_msg(improper_user.address()),
            &[Coin::new(10_000, improper_token), Coin::new(10_000, USDC_DENOM)],
            &improper_user
        ).is_err());

        assert!(vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &deposit_msg(improper_user.address()),
            &[Coin::new(10_000, USDC_DENOM), Coin::new(10_000, OSMO_DENOM), Coin::new(10_000, improper_token)],
            &improper_user
        ).is_err());

        vault_mockup.deposit(10_000, 10_000, &improper_user).unwrap();
    }

    #[test]
    fn timestamp_operations_wont_panic_for_large_values() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let seconds_before_rebalance = u32::MAX;
        let vault_mockup = VaultMockup::new_with_rebalancer(
            &pool_mockup,
            vault_params("2", "1.45", "0.55"),
            rebalancer_anyone("1", seconds_before_rebalance)
        );

        vault_mockup.deposit(10_000, 10_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user2).unwrap();
        pool_mockup.app.increase_time(1);
        assert!(vault_mockup.rebalance(&pool_mockup.user2).is_err());
        pool_mockup.app.increase_time((seconds_before_rebalance - 1).into());
        vault_mockup.rebalance(&pool_mockup.user2).unwrap();
    }

    #[test]
    fn vault_burning_smoke_test() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup.deposit(60_000, 60_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        pool_mockup.swap_osmo_for_usdc(&pool_mockup.user1, 30_000).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        vault_mockup.propose_new_admin(&pool_mockup.deployer, Some(&pool_mockup.user1)).unwrap();

        assert!(vault_mockup.burn_vault_admin(&pool_mockup.deployer).is_err());
        vault_mockup.propose_new_admin(&pool_mockup.deployer, None).unwrap();
        assert!(vault_mockup.burn_vault_admin(&pool_mockup.deployer).is_err());
        vault_mockup.change_vault_rebalancer(&pool_mockup.deployer, rebalancer_anyone("2", 123)).unwrap();
        assert!(vault_mockup.burn_vault_admin(&pool_mockup.deployer).is_err());
        vault_mockup.change_admin_fee(&pool_mockup.deployer, "0").unwrap();
        assert!(vault_mockup.burn_vault_admin(&pool_mockup.deployer).is_err());
        vault_mockup.admin_withdraw(&pool_mockup.deployer).unwrap();

        vault_mockup.burn_vault_admin(&pool_mockup.deployer).unwrap();
        assert!(vault_mockup.propose_new_admin(&pool_mockup.deployer, Some(&pool_mockup.user2)).is_err());
        assert!(vault_mockup.propose_new_admin(&pool_mockup.user2, Some(&pool_mockup.user1)).is_err());
        assert!(vault_mockup.accept_new_admin(&pool_mockup.user1).is_err());
    }

    #[test]
    fn vault_admin_proposing() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup.propose_new_admin(&pool_mockup.deployer, Some(&pool_mockup.user1)).unwrap();
        vault_mockup.propose_new_admin(&pool_mockup.deployer, None).unwrap();
        assert!(vault_mockup.accept_new_admin(&pool_mockup.user1).is_err());
        vault_mockup.propose_new_admin(&pool_mockup.deployer, Some(&pool_mockup.user1)).unwrap();
        vault_mockup.accept_new_admin(&pool_mockup.user1).unwrap();

        assert!(vault_mockup.change_vault_parameters(&pool_mockup.deployer, vault_params("2.5", "1.12", "0.1")).is_err());
        vault_mockup.change_vault_parameters(&pool_mockup.user1, vault_params("2.5", "1.12", "0.1")).unwrap();
    }

    #[test]
    fn proper_balances_for_out_of_range_vault_positions() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        vault_mockup.deposit(10_000, 10_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let limit_bals = vault_mockup.position_balances_query(PositionType::Limit);
        assert!(limit_bals.bal0_fees.is_zero());
        assert!(limit_bals.bal1_fees.is_zero());
        assert!(limit_bals.bal0.is_zero());
        // 5_000 tokens out of proportion, -1 atom.
        assert_eq!(limit_bals.bal1, Uint128::new(4999));
    }

    /// We will create a vault with only a limit position. Then the price will
    /// move enough to make the vault reserves balanced. Thus, at that point,
    /// rebalancing wont produce any limit positions. For this computation:
    /// 1. Observe that a limit position in [p/k, p] will only be balanced
    ///    if price moves to p/sqrt(k) (geometric mean). Dualy, a limit position
    ///    in [p, pk] will only be balanced if the price moves to p*sqrt(k).
    /// 2. Observe that we can trivially know the liquidity at any given range.
    ///    Because our liqudity pool will only have 2 positions, the computations
    ///    are even easier.
    /// 3. Finally, we can compute the reserve deltas sufficient to move the
    ///    price to our desired endpoints.
    #[test]
    fn from_limit_position_to_balanced_one() {
        let pool_mockup = PoolMockup::new_with_spread(200_000, 100_000, "0");
        let full_range_liquidity = pool_mockup
            .position_liquidity(pool_mockup.initial_position_id)
            .unwrap();

        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "2", "0.55"));
        vault_mockup.deposit(0, 50_000, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        let position_ids = vault_mockup.vault_state_query();
        assert!(position_ids.full_range_position_id.is_none());
        assert!(position_ids.base_position_id.is_none());
        assert!(position_ids.limit_position_id.is_some());

        let VaultParameters { limit_factor, .. } = vault_mockup.vault_parameters_query();

        let target_price = pool_mockup.price / limit_factor.0.sqrt();
        let limit_liquidity = pool_mockup
            .position_liquidity(position_ids.limit_position_id.unwrap())
            .unwrap();

        let liquidity = full_range_liquidity + limit_liquidity;
        let delta_x = liquidity * (
            Decimal::one()/target_price.sqrt() - Decimal::one()/pool_mockup.price.sqrt()
        );
        let delta_x = delta_x.to_uint_floor();

        // NOTE: We subtract 1 for atomic reasons.
        pool_mockup.swap_usdc_for_osmo(&pool_mockup.user2, delta_x.u128() - 1).unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        let position_ids = vault_mockup.vault_state_query();
        assert!(position_ids.full_range_position_id.is_some());
        assert!(position_ids.base_position_id.is_some());
        assert!(position_ids.limit_position_id.is_none());
    }
}
