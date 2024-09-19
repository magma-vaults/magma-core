use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Reply, Response,
    StdResult, Uint128
};
use cw20_base::contract::{query_balance, query_token_info};
use cw20_base::state::{MinterData, TokenInfo, TOKEN_INFO};
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::MsgCreatePositionResponse;

use crate::msg::
    QueryMsg
;
use crate::state::{ProtocolInfo, PROTOCOL_INFO};
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
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {

    let vault_info = VaultInfo::new(msg.vault_info.clone(), deps.as_ref())?;
    let vault_parameters = VaultParameters::new(msg.vault_parameters)?;
    let vault_state = VaultState::default();
    let protocol_info = ProtocolInfo::default();
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
    //            theyre proper during development.
    do_me! {
        VAULT_INFO.save(deps.storage, &vault_info)?;
        VAULT_PARAMETERS.save(deps.storage, &vault_parameters)?;
        VAULT_STATE.save(deps.storage, &vault_state)?;
        PROTOCOL_INFO.save(deps.storage, &protocol_info)?;
        TOKEN_INFO.save(deps.storage, &token_info)?;
    }.unwrap();

    Ok(Response::new())
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    use QueryMsg::*;
    match msg {
        VaultBalances {} => to_json_binary(&query::vault_balances(deps, &env)),
        PositionBalancesWithFees { position_type } => to_json_binary(
            &query::position_balances_with_fees(position_type, deps),
        ),
        CalcSharesAndUsableAmounts {
            for_amount0,
            for_amount1,
        } => to_json_binary(&query::calc_shares_and_usable_amounts(
            for_amount0,
            for_amount1,
            false,
            deps,
            &env,
        )),
        Balance { address } => to_json_binary(&query_balance(deps, address)?),
        VaultPositions {} => {
            // Invariant: Any state is present after instantiation.
            to_json_binary(&VAULT_STATE.load(deps.storage).unwrap())
        },
        TokenInfo {} => to_json_binary(&query_token_info(deps)?)
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
        Deposit(deposit_msg) => Ok(execute::deposit(deposit_msg, deps, env, info)?),
        Rebalance {} => Ok(execute::rebalance(deps, env)?),
        Withdraw(withdraw_msg) => Ok(execute::withdraw(withdraw_msg, deps, env, info)?),
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
    use std::{borrow::Borrow, rc::Rc, str::FromStr};

    use crate::{assert_approx_eq, constants::{MAX_TICK, MIN_TICK}, msg::{DepositMsg, PositionBalancesWithFeesResponse, VaultBalancesResponse, VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg, WithdrawMsg}, state::PositionType, utils::price_function_inv};

    use super::*;
    use cosmwasm_std::{coin, testing::mock_dependencies, Addr, Api, BankQuery, Coin, Decimal};
    use cw20::BalanceResponse;
    use osmosis_std::types::{cosmos::bank::v1beta1::QueryBalanceRequest, cosmwasm::wasm::v1::MsgExecuteContractResponse, osmosis::{
        concentratedliquidity::v1beta1::{
            CreateConcentratedLiquidityPoolsProposal, MsgCreatePosition, PoolRecord, PositionByIdRequest
        }, poolmanager::v1beta1::{MsgSwapExactAmountIn, SwapAmountInRoute}}}
    ;
    use osmosis_test_tube::{Account, Bank, ConcentratedLiquidity, ExecuteResponse, GovWithAppAccess, Module, OsmosisTestApp, PoolManager, SigningAccount, Wasm};

    const USDC_DENOM: &str = "ibc/DE6792CF9E521F6AD6E9A4BDF6225C9571A3B74ACC0A529F92BC5122A39D2E58";
    const OSMO_DENOM: &str = "uosmo";

    struct PoolMockup {
        pool_id: u64,
        app: OsmosisTestApp,
        deployer: SigningAccount,
        user1: SigningAccount,
        user2: SigningAccount,
        _price: Decimal,
    }

    impl PoolMockup {
        fn new(x_bal: u128, y_bal: u128) -> Self {
            let app = OsmosisTestApp::new();
            
            let init_coins = &[
                Coin::new(1_000_000_000_000u128, USDC_DENOM),
                Coin::new(1_000_000_000_000u128, OSMO_DENOM),
            ];

            let mut accounts = app.init_accounts(init_coins, 3).unwrap().into_iter();
            let deployer = accounts.next().unwrap();
            let user1 = accounts.next().unwrap();
            let user2 = accounts.next().unwrap();

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
                        spread_factor: Decimal::from_str("0.01").unwrap().atomics().into()
                    }]
                },
                deployer.address(),
                &deployer,
            )
            .unwrap();

            // NOTE: Could fail if we test multiple pools.
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

            // NOTE: Could fail if we test multiple positions.
            assert_eq!(position_res.position_id, 1);

            let _price = Decimal::new(y_bal.into()) / Decimal::new(x_bal.into());

            Self {
                pool_id, app, deployer, user1, user2, _price
            }
        }

        fn swap_osmo_for_usdc(&self, from: &SigningAccount, osmo_in: u128) -> anyhow::Result<Uint128> {
            let pm = PoolManager::new(&self.app);
            let usdc_got = pm.swap_exact_amount_in(
                MsgSwapExactAmountIn {
                    sender: from.address(),
                    routes: vec![SwapAmountInRoute {
                        pool_id: self.pool_id,
                        token_out_denom: USDC_DENOM.into(),
                    }],
                    token_in: Some(Coin::new(osmo_in, OSMO_DENOM).into()),
                    token_out_min_amount: "1".into(),
                },
                from
            )
                .map(|x| x.data.token_out_amount)
                .map(|amount| Uint128::from_str(&amount).unwrap());

            Ok(usdc_got?)
        }

        fn swap_usdc_for_osmo(&self, from: &SigningAccount, usdc_in: u128) -> anyhow::Result<Uint128> {
            let pm = PoolManager::new(&self.app);
            let usdc_got = pm.swap_exact_amount_in(
                MsgSwapExactAmountIn {
                    sender: from.address(),
                    routes: vec![SwapAmountInRoute {
                        pool_id: self.pool_id,
                        token_out_denom: OSMO_DENOM.into(),
                    }],
                    token_in: Some(Coin::new(usdc_in, USDC_DENOM).into()),
                    token_out_min_amount: "1".into(),
                },
                from
            )
                .map(|x| x.data.token_out_amount)
                .map(|amount| Uint128::from_str(&amount).unwrap());

            Ok(usdc_got?)
        }

        fn osmo_balance_query(&self, address: &str) -> Uint128 {
            let bank = Bank::new(&self.app);
            let amount = bank.query_balance(&QueryBalanceRequest{
                address: address.into(),
                denom: OSMO_DENOM.into()
            }).unwrap().balance.unwrap().amount;
            Uint128::from_str(&amount).unwrap()
        }

        fn usdc_balance_query(&self, address: &str) -> Uint128 {
            let bank = Bank::new(&self.app);
            let amount = bank.query_balance(&QueryBalanceRequest{
                address: address.into(),
                denom: USDC_DENOM.into()
            }).unwrap().balance.unwrap().amount;
            Uint128::from_str(&amount).unwrap()
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

    struct VaultMockup<'a> {
        vault_addr: Addr,
        pool_mockup: &'a PoolMockup,
        wasm: Wasm<'a, OsmosisTestApp>
    }

    impl VaultMockup<'_> {
        fn new<'a>(pool_info: &PoolMockup, params: VaultParametersInstantiateMsg) -> VaultMockup {
            let wasm = Wasm::new(&pool_info.app);
            let code_id = store_vaults_code(&wasm, &pool_info.deployer);
            let api = mock_dependencies().api;

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

            let vault_addr = api.addr_validate(&vault_addr).unwrap();

            VaultMockup { vault_addr, pool_mockup: pool_info, wasm }
        }

        fn deposit(
            &self,
            amount0: u128,
            amount1: u128,
            from: &SigningAccount
        ) -> anyhow::Result<ExecuteResponse<MsgExecuteContractResponse>> {
            let execute_msg = &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(amount0),
                amount1: Uint128::new(amount1),
                amount0_min: Uint128::zero(),
                amount1_min: Uint128::zero(),
                to: from.address()
            });

            let coin0 = Coin::new(amount0, USDC_DENOM);
            let coin1 = Coin::new(amount1, OSMO_DENOM);

            if amount0 == 0 && amount1 == 0 {
                unimplemented!()
            } else if amount0 == 0 {
                Ok(self.wasm.execute(
                    &self.vault_addr.to_string(),
                    execute_msg,
                    &[coin1],
                    from
                )?)
            } else if amount1 == 0 {
                Ok(self.wasm.execute(
                    &self.vault_addr.to_string(),
                    execute_msg,
                    &[coin0],
                    from
                )?)
            } else {
                Ok(self.wasm.execute(
                    &self.vault_addr.to_string(),
                    execute_msg,
                    &[coin0, coin1],
                    from
                )?)
            }
        }

        fn rebalance(
            &self,
            from: &SigningAccount
        ) -> anyhow::Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                &self.vault_addr.to_string(), &ExecuteMsg::Rebalance {}, &[], from
            )?)
        }

        fn withdraw(
            &self,
            shares: Uint128,
            from: &SigningAccount
        ) -> anyhow::Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                &self.vault_addr.to_string(),
                &ExecuteMsg::Withdraw(WithdrawMsg{
                    shares,
                    amount0_min: Uint128::zero(),
                    amount1_min: Uint128::zero(),
                    to: from.address().into()
                }),
                &[],
                from
            )?)
        }

        fn vault_balances_query(&self) -> VaultBalancesResponse {
            self.wasm.query(
                &self.vault_addr.to_string(),
                &QueryMsg::VaultBalances { }
            ).unwrap()
        }

        fn position_balances_query(&self, position_type: PositionType) -> PositionBalancesWithFeesResponse {
            self.wasm.query(
                &self.vault_addr.to_string(),
                &QueryMsg::PositionBalancesWithFees { position_type },
            ).unwrap()
        }

        fn shares_query(&self, address: &str) -> Uint128 {
            let res: cw20::BalanceResponse = self.wasm.query(
                &self.vault_addr.to_string(),
                &QueryMsg::Balance { address: address.into() }
            ).unwrap();
            res.balance
        }

        fn vault_state_query(&self) -> VaultState {
            self.wasm.query(
                &self.vault_addr.to_string(),
                &QueryMsg::VaultPositions {}
            ).unwrap()
        }

        /* TODO
        let withdraw_vault_addr = vault_addr.clone();
        let withdraw_wasm = Rc::clone(&wasm);
        let withdraw = move || {
            withdraw_wasm.execute(
                &rebalance_vault_addr.to_string(), 
                &ExecuteMsg::Withdraw(
                    WithdrawMsg {
                        shares: shares_got.balance,
                        amount0_min: vault_balances_before_withdrawal.bal0,
                        amount1_min: vault_balances_before_withdrawal.bal1,
                        to: pool_info.deployer.address()
                    }
                ),
                &[],
                &pool_info.deployer
            ).unwrap();
        };
        */
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
    fn normal_rebalances() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        vault_mockup.deposit(1_000, 1_500, &pool_mockup.user1).unwrap();
        let bals = vault_mockup.vault_balances_query();
        assert_eq!(bals.bal0.u128(), 1_000);
        assert_eq!(bals.bal1.u128(), 1_500);
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();

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
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        vault_mockup.deposit(1_000, 1_500, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();
    }

    #[test]
    fn rebalance_in_proportion() {
        let pool_balance0 = 100_000;
        let pool_balance1 = 200_000;
        let pool_mockup = PoolMockup::new(pool_balance0, pool_balance1);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });
        
        vault_mockup.deposit(pool_balance0/2, pool_balance1/2, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();
        assert!(vault_mockup.vault_state_query().limit_position_id.is_none());
        assert!(vault_mockup.vault_state_query().full_range_position_id.is_some());
        assert!(vault_mockup.vault_state_query().base_position_id.is_some());
    }

    #[test]
    fn only_limit_rebalance() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        vault_mockup.deposit(42, 0, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();

        // Dual case
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        vault_mockup.deposit(0, 42, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();

        // Combined case
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        vault_mockup.deposit(42, 0, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();
        assert!(vault_mockup.vault_state_query().limit_position_id.is_some());
        assert!(vault_mockup.vault_state_query().full_range_position_id.is_none());
        assert!(vault_mockup.vault_state_query().base_position_id.is_none());

        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();
        // FIXME: See issue #1.
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
    fn rebalance_after_price_change() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        let (vault_x, vault_y) = (1_000, 1_000);
        vault_mockup.deposit(vault_x, vault_y, &pool_mockup.user1).unwrap();
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();

        let usdc_got = pool_mockup.swap_osmo_for_usdc(&pool_mockup.user1, vault_y/10).unwrap();
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();
        pool_mockup.swap_usdc_for_osmo(&pool_mockup.user1, usdc_got.into()).unwrap();
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();
    }

    #[test]
    fn out_of_range_vault_positions_test() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        let (vault_x, vault_y) = (1_000, 1_500);
        vault_mockup.deposit(vault_x, vault_y, &pool_mockup.user1).unwrap();
        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.rebalance(&pool_mockup.user1).unwrap();
        pool_mockup.swap_usdc_for_osmo(&pool_mockup.user1, 50_000).unwrap();
        vault_mockup.withdraw(shares_got, &pool_mockup.user1).unwrap();
    }

    #[test]
    fn withdraw_without_rebalances() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_mockup = PoolMockup::new(pool_x, pool_y);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        let (vault_x, vault_y) = (1_000, 1_500);
        vault_mockup.deposit(vault_x, vault_y, &pool_mockup.user1).unwrap();
        let vault_bals_before_withdrawal = vault_mockup.vault_balances_query();
        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        assert!(vault_mockup.withdraw(shares_got, &pool_mockup.user2).is_err());
        vault_mockup.withdraw(shares_got, &pool_mockup.user1).unwrap();
        let vault_bals_after_withdrawal = vault_mockup.vault_balances_query();

        assert_eq!(vault_bals_before_withdrawal.bal0, Uint128::new(vault_x));
        assert_eq!(vault_bals_before_withdrawal.bal1, Uint128::new(vault_y));
        assert!(vault_bals_after_withdrawal.bal0.is_zero());
        assert!(vault_bals_after_withdrawal.bal1.is_zero());
    }


    #[test]
    fn withdraw_limit_without_rebalances() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_mockup = PoolMockup::new(pool_x, pool_y);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        let (vault_x, vault_y) = (0, 6969);
        vault_mockup.deposit(vault_x, vault_y, &pool_mockup.user1).unwrap();
        
        assert!(vault_mockup.vault_balances_query().bal0.is_zero());
        assert_eq!(vault_mockup.vault_balances_query().bal1, Uint128::new(vault_y));

        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares_got, &pool_mockup.user1).unwrap();

        assert!(vault_mockup.vault_balances_query().bal0.is_zero());
        assert!(vault_mockup.vault_balances_query().bal1.is_zero());
    }

    #[test]
    fn withdraw_with_min_amounts() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_mockup = PoolMockup::new(pool_x, pool_y);
        let vault_mockup = VaultMockup::new(&pool_mockup, VaultParametersInstantiateMsg {
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into(),
        });

        let (vault_x, vault_y) = (1_000, 1_500);

        let improper_deposit = vault_mockup.wasm.execute(
            &vault_mockup.vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
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
            &vault_mockup.vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
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
            &vault_mockup.vault_addr.to_string(), 
            &ExecuteMsg::Withdraw(
                WithdrawMsg {
                    shares: shares_got,
                    amount0_min: vault_balances_before_withdrawal.bal0 + Uint128::one(),
                    amount1_min: vault_balances_before_withdrawal.bal1 + Uint128::one(),
                    to: pool_mockup.user1.address()
                }
            ),
            &[],
            &pool_mockup.user1
        );
        assert!(improper_withdrawal.is_err());

        vault_mockup.wasm.execute(
            &vault_mockup.vault_addr.to_string(), 
            &ExecuteMsg::Withdraw(
                WithdrawMsg {
                    shares: shares_got,
                    amount0_min: vault_balances_before_withdrawal.bal0,
                    amount1_min: vault_balances_before_withdrawal.bal1,
                    to: pool_mockup.user1.address()
                }
            ),
            &[],
            &pool_mockup.user1
        ).unwrap();

        let vault_balances_after_withdrawal = vault_mockup.vault_balances_query();

        assert!(vault_balances_after_withdrawal.bal0.is_zero());
        assert!(vault_balances_after_withdrawal.bal1.is_zero());
        assert!(vault_balances_after_withdrawal.protocol_unclaimed_fees0.is_zero());
        assert!(vault_balances_after_withdrawal.protocol_unclaimed_fees1.is_zero());
    }
}
