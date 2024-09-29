use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Reply, Response,
    StdResult, Uint128,
};
use cw20_base::contract::{query_balance, query_token_info};
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
    if !info.funds.is_empty() {
        return Err(ContractError::NonPayable("instantiate".into()));
    }

    let vault_info = VaultInfo::new(msg.vault_info.clone(), deps.as_ref())?;
    let vault_parameters = VaultParameters::new(msg.vault_parameters.clone())?;
    let vault_state = VaultState::default();
    let protocol_info = FeesInfo::new(msg.vault_parameters.admin_fee, &vault_info)?;
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
        FEES_INFO.save(deps.storage, &protocol_info)?;
        FUNDS_INFO.save(deps.storage, &funds_info)?;
        TOKEN_INFO.save(deps.storage, &token_info)?;
    }
    .unwrap();

    Ok(Response::new())
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    use QueryMsg::*;
    match msg {
        VaultBalances {} => to_json_binary(&query::vault_balances(deps)),
        PositionBalancesWithFees { position_type } => {
            to_json_binary(&query::position_balances_with_fees(position_type, deps))
        }
        CalcSharesAndUsableAmounts {
            for_amount0,
            for_amount1,
        } => to_json_binary(&query::calc_shares_and_usable_amounts(
            for_amount0,
            for_amount1,
            deps,
        )),
        Balance { address } => to_json_binary(&query_balance(deps, address)?),
        // Invariant: Any state is present after instantiation.
        VaultState {} => to_json_binary(&VAULT_STATE.load(deps.storage).unwrap()),
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
        return Err(ContractError::NonPayable(format!("{:?}", msg)));
    }

    match msg {
        Deposit(deposit_msg) => Ok(execute::deposit(deposit_msg, deps, env, info)?),
        Rebalance {} => Ok(execute::rebalance(deps, env, info)?),
        Withdraw(withdraw_msg) => Ok(execute::withdraw(withdraw_msg, deps, env, info)?),
        WithdrawProtocolFees {} => Ok(execute::withdraw_protocol_fees(deps, info)?),
        WithdrawAdminFees {} => Ok(execute::withdraw_admin_fees(deps, info)?),
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
mod test {
    use std::str::FromStr;

    use crate::{
        assert_approx_eq,
        constants::{MAX_TICK, MIN_LIQUIDITY, MIN_TICK},
        msg::{
            DepositMsg, PositionBalancesWithFeesResponse, VaultBalancesResponse,
            VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg,
            WithdrawMsg,
        },
        state::{PositionType, ProtocolFee},
        utils::price_function_inv,
    };

    use super::*;
    use cosmwasm_std::{coin, testing::mock_dependencies, Addr, Api, Coin, Decimal};
    use osmosis_std::types::{
        cosmos::bank::v1beta1::QueryBalanceRequest,
        cosmwasm::wasm::v1::MsgExecuteContractResponse,
        osmosis::{
            concentratedliquidity::v1beta1::{
                CreateConcentratedLiquidityPoolsProposal, MsgCreatePosition, PoolRecord,
            },
            poolmanager::v1beta1::{MsgSwapExactAmountIn, SwapAmountInRoute},
        },
    };
    use osmosis_test_tube::{
        Account, Bank, ConcentratedLiquidity, ExecuteResponse, GovWithAppAccess, Module,
        OsmosisTestApp, PoolManager, SigningAccount, Wasm,
    };

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
        fn new(usdc_in: u128, osmo_in: u128) -> Self {
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
                        spread_factor: Decimal::from_str("0.01").unwrap().atomics().into(),
                    }],
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
                            Coin::new(usdc_in, USDC_DENOM).into(),
                            Coin::new(osmo_in, OSMO_DENOM).into(),
                        ],
                        token_min_amount0: usdc_in.to_string(),
                        token_min_amount1: osmo_in.to_string(),
                    },
                    &deployer,
                )
                .unwrap()
                .data;

            // NOTE: Could fail if we test multiple positions.
            assert_eq!(position_res.position_id, 1);

            let _price = Decimal::new(osmo_in.into()) / Decimal::new(usdc_in.into());

            Self {
                pool_id,
                app,
                deployer,
                user1,
                user2,
                _price,
            }
        }

        fn swap_osmo_for_usdc(
            &self,
            from: &SigningAccount,
            osmo_in: u128,
        ) -> anyhow::Result<Uint128> {
            let pm = PoolManager::new(&self.app);
            let usdc_got = pm
                .swap_exact_amount_in(
                    MsgSwapExactAmountIn {
                        sender: from.address(),
                        routes: vec![SwapAmountInRoute {
                            pool_id: self.pool_id,
                            token_out_denom: USDC_DENOM.into(),
                        }],
                        token_in: Some(Coin::new(osmo_in, OSMO_DENOM).into()),
                        token_out_min_amount: "1".into(),
                    },
                    from,
                )
                .map(|x| x.data.token_out_amount)
                .map(|amount| Uint128::from_str(&amount).unwrap());

            Ok(usdc_got?)
        }

        fn swap_usdc_for_osmo(
            &self,
            from: &SigningAccount,
            usdc_in: u128,
        ) -> anyhow::Result<Uint128> {
            let pm = PoolManager::new(&self.app);
            let usdc_got = pm
                .swap_exact_amount_in(
                    MsgSwapExactAmountIn {
                        sender: from.address(),
                        routes: vec![SwapAmountInRoute {
                            pool_id: self.pool_id,
                            token_out_denom: OSMO_DENOM.into(),
                        }],
                        token_in: Some(Coin::new(usdc_in, USDC_DENOM).into()),
                        token_out_min_amount: "1".into(),
                    },
                    from,
                )
                .map(|x| x.data.token_out_amount)
                .map(|amount| Uint128::from_str(&amount).unwrap());

            Ok(usdc_got?)
        }

        fn osmo_balance_query(&self, address: &str) -> Uint128 {
            let bank = Bank::new(&self.app);
            let amount = bank
                .query_balance(&QueryBalanceRequest {
                    address: address.into(),
                    denom: OSMO_DENOM.into(),
                })
                .unwrap()
                .balance
                .unwrap()
                .amount;
            Uint128::from_str(&amount).unwrap()
        }

        fn usdc_balance_query(&self, address: &str) -> Uint128 {
            let bank = Bank::new(&self.app);
            let amount = bank
                .query_balance(&QueryBalanceRequest {
                    address: address.into(),
                    denom: USDC_DENOM.into(),
                })
                .unwrap()
                .balance
                .unwrap()
                .amount;
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

    fn vault_params(base: &str, limit: &str, full: &str) -> VaultParametersInstantiateMsg {
        VaultParametersInstantiateMsg {
            full_range_weight: full.into(),
            base_factor: base.into(),
            limit_factor: limit.into(),
            admin_fee: ProtocolFee::default().0 .0.to_string(),
        }
    }

    struct VaultMockup<'a> {
        vault_addr: Addr,
        wasm: Wasm<'a, OsmosisTestApp>,
    }

    impl VaultMockup<'_> {
        fn new(pool_info: &PoolMockup, params: VaultParametersInstantiateMsg) -> VaultMockup {
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

            VaultMockup { vault_addr, wasm }
        }

        fn deposit(
            &self,
            amount0: u128,
            amount1: u128,
            from: &SigningAccount,
        ) -> anyhow::Result<ExecuteResponse<MsgExecuteContractResponse>> {
            let execute_msg = &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(amount0),
                amount1: Uint128::new(amount1),
                amount0_min: Uint128::zero(),
                amount1_min: Uint128::zero(),
                to: from.address(),
            });

            let coin0 = Coin::new(amount0, USDC_DENOM);
            let coin1 = Coin::new(amount1, OSMO_DENOM);

            if amount0 == 0 && amount1 == 0 {
                unimplemented!()
            } else if amount0 == 0 {
                Ok(self
                    .wasm
                    .execute(self.vault_addr.as_ref(), execute_msg, &[coin1], from)?)
            } else if amount1 == 0 {
                Ok(self
                    .wasm
                    .execute(self.vault_addr.as_ref(), execute_msg, &[coin0], from)?)
            } else {
                Ok(self.wasm.execute(
                    self.vault_addr.as_ref(),
                    execute_msg,
                    &[coin0, coin1],
                    from,
                )?)
            }
        }

        fn rebalance(
            &self,
            from: &SigningAccount,
        ) -> anyhow::Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::Rebalance {},
                &[],
                from,
            )?)
        }

        fn withdraw(
            &self,
            shares: Uint128,
            from: &SigningAccount,
        ) -> anyhow::Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::Withdraw(WithdrawMsg {
                    shares,
                    amount0_min: Uint128::zero(),
                    amount1_min: Uint128::zero(),
                    to: from.address(),
                }),
                &[],
                from,
            )?)
        }

        fn admin_withdraw(
            &self,
            from: &SigningAccount,
        ) -> anyhow::Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::WithdrawAdminFees {},
                &[],
                from,
            )?)
        }

        fn vault_balances_query(&self) -> VaultBalancesResponse {
            self.wasm
                .query(self.vault_addr.as_ref(), &QueryMsg::VaultBalances {})
                .unwrap()
        }

        fn position_balances_query(
            &self,
            position_type: PositionType,
        ) -> PositionBalancesWithFeesResponse {
            self.wasm
                .query(
                    self.vault_addr.as_ref(),
                    &QueryMsg::PositionBalancesWithFees { position_type },
                )
                .unwrap()
        }

        fn token_info_query(&self) -> TokenInfo {
            self.wasm
                .query(self.vault_addr.as_ref(), &QueryMsg::TokenInfo {})
                .unwrap()
        }

        fn shares_query(&self, address: &str) -> Uint128 {
            let res: cw20::BalanceResponse = self
                .wasm
                .query(
                    self.vault_addr.as_ref(),
                    &QueryMsg::Balance {
                        address: address.into(),
                    },
                )
                .unwrap();
            res.balance
        }

        fn vault_state_query(&self) -> VaultState {
            self.wasm
                .query(self.vault_addr.as_ref(), &QueryMsg::VaultState {})
                .unwrap()
        }

        fn vault_fees_query(&self) -> FeesInfo {
            self.wasm
                .query(self.vault_addr.as_ref(), &QueryMsg::FeesInfo {})
                .unwrap()
        }
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
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup
            .deposit(1_000, 1_500, &pool_mockup.user1)
            .unwrap();
        let bals = vault_mockup.vault_balances_query();
        assert_eq!(bals.bal0.u128(), 1_000);
        assert_eq!(bals.bal1.u128(), 1_500);
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        let full_range_position = vault_mockup.position_balances_query(PositionType::FullRange);

        // \[
        //   x_0 = \frac{\sqrt k X w}{\sqrt k - 1 + w}
        //       = \frac{\sqrt 2 \cdot 750 \cdot 0.55}{\sqrt 2 - 1 + 0.55 }
        //       \approx 605$
        // \]
        assert_approx_eq!(full_range_position.bal0, Uint128::new(605), Uint128::new(5));
        // \[ y_0 = x_0 p \]
        assert_approx_eq!(
            full_range_position.bal1,
            Uint128::new(605 * 2),
            Uint128::new(5)
        );
    }

    #[test]
    fn normal_rebalance_dual() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup
            .deposit(1_000, 1_500, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
    }

    #[test]
    fn rebalance_in_proportion() {
        let pool_balance0 = 100_000;
        let pool_balance1 = 200_000;
        let pool_mockup = PoolMockup::new(pool_balance0, pool_balance1);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup
            .deposit(pool_balance0 / 2, pool_balance1 / 2, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        assert!(vault_mockup.vault_state_query().limit_position_id.is_none());
        assert!(vault_mockup
            .vault_state_query()
            .full_range_position_id
            .is_some());
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
        assert!(vault_mockup
            .vault_state_query()
            .full_range_position_id
            .is_none());
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

        vault_mockup
            .deposit(10_000, 20_000, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();

        vault_mockup
            .deposit(10_000, 20_000, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();
    }

    #[test]
    fn full_liquidation() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup
            .deposit(10_000, 25_000, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();

        vault_mockup
            .deposit(10_000, 25_000, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();
    }

    #[test]
    fn no_limit_position_on_rebalance() {
        // Initialize pool and vault with specific parameters
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        // Deposit amounts calculated to get balances in proportion
        // Replace the placeholders with actual calculated values
        let desired_amount0 = 10_000;
        let desired_amount1 = 20_000; // Adjust as needed

        vault_mockup
            .deposit(desired_amount0, desired_amount1, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        // Add assertions to verify the expected outcome
        // For example:
        assert!(vault_mockup.vault_state_query().limit_position_id.is_none());
        assert!(vault_mockup
            .vault_state_query()
            .full_range_position_id
            .is_some());
        assert!(vault_mockup.vault_state_query().base_position_id.is_some());
    }

    #[test]
    fn rebalance_after_price_change() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (10_000, 10_000);
        vault_mockup
            .deposit(vault_x, vault_y, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        let usdc_got = pool_mockup
            .swap_osmo_for_usdc(&pool_mockup.user1, vault_y / 10)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        pool_mockup
            .swap_usdc_for_osmo(&pool_mockup.user1, usdc_got.into())
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
    }

    #[test]
    fn out_of_range_vault_positions_test() {
        let pool_mockup = PoolMockup::new(100_000, 200_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (10_000, 15_000);
        vault_mockup
            .deposit(vault_x, vault_y, &pool_mockup.user1)
            .unwrap();
        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        pool_mockup
            .swap_usdc_for_osmo(&pool_mockup.user1, 50_000)
            .unwrap();
        vault_mockup
            .withdraw(shares_got, &pool_mockup.user1)
            .unwrap();
    }

    #[test]
    fn withdraw_without_rebalances() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_mockup = PoolMockup::new(pool_x, pool_y);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (10_000, 15_000);
        vault_mockup
            .deposit(vault_x, vault_y, &pool_mockup.user1)
            .unwrap();
        let vault_bals_before_withdrawal = vault_mockup.vault_balances_query();
        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        assert!(vault_mockup
            .withdraw(shares_got, &pool_mockup.user2)
            .is_err());
        vault_mockup
            .withdraw(shares_got, &pool_mockup.user1)
            .unwrap();
        let vault_bals_after_withdrawal = vault_mockup.vault_balances_query();

        assert_eq!(vault_bals_before_withdrawal.bal0, Uint128::new(vault_x));
        assert_eq!(vault_bals_before_withdrawal.bal1, Uint128::new(vault_y));
        assert_approx_eq!(
            vault_bals_after_withdrawal.bal0,
            Uint128::zero(),
            MIN_LIQUIDITY + Uint128::one()
        );
        assert_approx_eq!(
            vault_bals_after_withdrawal.bal1,
            Uint128::zero(),
            MIN_LIQUIDITY + Uint128::one()
        );
    }

    #[test]
    fn withdraw_limit_without_rebalances() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_mockup = PoolMockup::new(pool_x, pool_y);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        let (vault_x, vault_y) = (0, 6969);
        vault_mockup
            .deposit(vault_x, vault_y, &pool_mockup.user1)
            .unwrap();

        assert!(vault_mockup.vault_balances_query().bal0.is_zero());
        assert_eq!(
            vault_mockup.vault_balances_query().bal1,
            Uint128::new(vault_y)
        );

        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup
            .withdraw(shares_got, &pool_mockup.user1)
            .unwrap();

        assert!(vault_mockup.vault_balances_query().bal0.is_zero());
        assert_approx_eq!(
            vault_mockup.vault_balances_query().bal1,
            Uint128::zero(),
            MIN_LIQUIDITY + Uint128::one()
        );
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
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
                amount0_min: Uint128::new(vault_x) + Uint128::one(),
                amount1_min: Uint128::new(vault_y) + Uint128::one(),
                to: pool_mockup.user1.address(),
            }),
            &[coin(vault_x, USDC_DENOM), coin(vault_y, OSMO_DENOM)],
            &pool_mockup.user1,
        );
        assert!(improper_deposit.is_err());

        vault_mockup
            .wasm
            .execute(
                vault_mockup.vault_addr.as_ref(),
                &ExecuteMsg::Deposit(DepositMsg {
                    amount0: Uint128::new(vault_x),
                    amount1: Uint128::new(vault_y),
                    amount0_min: Uint128::new(vault_x),
                    amount1_min: Uint128::new(vault_y),
                    to: pool_mockup.user1.address(),
                }),
                &[coin(vault_x, USDC_DENOM), coin(vault_y, OSMO_DENOM)],
                &pool_mockup.user1,
            )
            .unwrap();

        let vault_balances_before_withdrawal = vault_mockup.vault_balances_query();
        let shares_got = vault_mockup.shares_query(&pool_mockup.user1.address());

        let improper_withdrawal = vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &ExecuteMsg::Withdraw(WithdrawMsg {
                shares: shares_got,
                amount0_min: vault_balances_before_withdrawal.bal0 - MIN_LIQUIDITY,
                amount1_min: vault_balances_before_withdrawal.bal1 - MIN_LIQUIDITY,
                to: pool_mockup.user1.address(),
            }),
            &[],
            &pool_mockup.user1,
        );
        assert!(improper_withdrawal.is_err());

        vault_mockup
            .wasm
            .execute(
                vault_mockup.vault_addr.as_ref(),
                &ExecuteMsg::Withdraw(WithdrawMsg {
                    shares: shares_got,
                    amount0_min: vault_balances_before_withdrawal.bal0
                        - MIN_LIQUIDITY
                        - Uint128::one(),
                    amount1_min: vault_balances_before_withdrawal.bal1
                        - MIN_LIQUIDITY
                        - Uint128::one(),
                    to: pool_mockup.user1.address(),
                }),
                &[],
                &pool_mockup.user1,
            )
            .unwrap();

        let vault_balances_after_withdrawal = vault_mockup.vault_balances_query();

        assert_approx_eq!(
            vault_balances_after_withdrawal.bal0,
            Uint128::zero(),
            MIN_LIQUIDITY + Uint128::one()
        );
        assert_approx_eq!(
            vault_balances_after_withdrawal.bal1,
            Uint128::zero(),
            MIN_LIQUIDITY + Uint128::one()
        );
        assert!(vault_balances_after_withdrawal
            .protocol_unclaimed_fees0
            .is_zero());
        assert!(vault_balances_after_withdrawal
            .protocol_unclaimed_fees1
            .is_zero());
    }

    #[test]
    fn fees_withdrawals_on_rebalance() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        vault_mockup
            .deposit(100_000, 50_000, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        pool_mockup
            .swap_osmo_for_usdc(&pool_mockup.user2, 20_000)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();

        let fees = vault_mockup.vault_fees_query();
        assert!(fees.admin_tokens0_owned.is_zero());
        assert!(!fees.admin_tokens1_owned.is_zero());
        assert!(fees.protocol_tokens0_owned.is_zero());
        assert!(!fees.protocol_tokens1_owned.is_zero());

        assert!(vault_mockup.admin_withdraw(&pool_mockup.user1).is_err());
        assert!(vault_mockup.admin_withdraw(&pool_mockup.user2).is_err());
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
        vault_mockup
            .deposit(100_000, 50_000, &pool_mockup.user1)
            .unwrap();
        vault_mockup.rebalance(&pool_mockup.deployer).unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());

        pool_mockup
            .swap_osmo_for_usdc(&pool_mockup.user2, 20_000)
            .unwrap();
        vault_mockup.withdraw(shares, &pool_mockup.user1).unwrap();

        let fees = vault_mockup.vault_fees_query();
        assert!(fees.admin_tokens0_owned.is_zero());
        assert!(!fees.admin_tokens1_owned.is_zero());
        assert!(fees.protocol_tokens0_owned.is_zero());
        assert!(!fees.protocol_tokens1_owned.is_zero());

        assert!(vault_mockup.admin_withdraw(&pool_mockup.user1).is_err());
        assert!(vault_mockup.admin_withdraw(&pool_mockup.user2).is_err());
        let x = vault_mockup.admin_withdraw(&pool_mockup.deployer).unwrap();
        println!("{:?}", x);
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
        assert!(vault_mockup
            .withdraw(Decimal::one().atomics(), &pool_mockup.deployer)
            .is_err());
        assert!(vault_mockup
            .withdraw(Uint128::zero(), &pool_mockup.deployer)
            .is_err());
    }

    #[test]
    fn cant_manipulate_contract_balances_in_unintended_ways() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));
        let should_err = vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(50_000),
                amount1: Uint128::new(50_000),
                amount0_min: Uint128::zero(),
                amount1_min: Uint128::zero(),
                to: pool_mockup.user1.address(),
            }),
            &[coin(50_000, USDC_DENOM), coin(50_001, OSMO_DENOM)],
            &pool_mockup.user1,
        );

        assert!(should_err.is_err());
        vault_mockup
            .deposit(50_000, 50_000, &pool_mockup.user1)
            .unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());

        let should_err = vault_mockup.wasm.execute(
            vault_mockup.vault_addr.as_ref(),
            &ExecuteMsg::Withdraw(WithdrawMsg {
                shares,
                amount0_min: Uint128::zero(),
                amount1_min: Uint128::zero(),
                to: pool_mockup.user1.address(),
            }),
            &[coin(1000, USDC_DENOM)],
            &pool_mockup.user1,
        );
        assert!(should_err.is_err());
    }

    #[test]
    fn min_liquidity_attack() {
        let pool_mockup = PoolMockup::new(200_000, 100_000);
        let vault_mockup = VaultMockup::new(&pool_mockup, vault_params("2", "1.45", "0.55"));

        vault_mockup
            .deposit(10_000, 10_000, &pool_mockup.user1)
            .unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user1.address());
        vault_mockup
            .withdraw(shares - Uint128::one(), &pool_mockup.user1)
            .unwrap();

        vault_mockup
            .deposit(10_000, 10_000, &pool_mockup.user2)
            .unwrap();
        let shares = vault_mockup.shares_query(&pool_mockup.user2.address());
        vault_mockup
            .withdraw(shares - Uint128::one(), &pool_mockup.user2)
            .unwrap();
    }
}
