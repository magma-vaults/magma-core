#[cfg(any(test, feature = "fuzzing"))]
pub mod mock {
    use anyhow::Result;
    use std::str::FromStr;

    use cosmwasm_std::{testing::mock_dependencies, Addr, Api, Coin, Decimal, Uint128};
    use cw20_base::state::TokenInfo;
    use osmosis_std::types::{
        cosmos::bank::v1beta1::QueryBalanceRequest,
        cosmwasm::wasm::v1::MsgExecuteContractResponse,
        osmosis::{
            concentratedliquidity::v1beta1::{
                CreateConcentratedLiquidityPoolsProposal, FullPositionBreakdown, MsgCreatePosition,
                PoolRecord, PositionByIdRequest,
            },
            poolmanager::v1beta1::{MsgSwapExactAmountIn, SwapAmountInRoute},
        },
    };
    use osmosis_test_tube::{
        Account, Bank, ConcentratedLiquidity, ExecuteResponse, GovWithAppAccess, Module,
        OsmosisTestApp, PoolManager, SigningAccount, Wasm,
    };

    use crate::{
        constants::{MAX_TICK, MIN_TICK, TWAP_SECONDS, VAULT_CREATION_COST_DENOM},
        msg::{
            DepositMsg, ExecuteMsg, InstantiateMsg, PositionBalancesWithFeesResponse, QueryMsg,
            VaultBalancesResponse, VaultInfoInstantiateMsg, VaultParametersInstantiateMsg,
            VaultRebalancerInstantiateMsg, WithdrawMsg,
        },
        state::{
            FeesInfo, PositionType, ProtocolFee, VaultCreationCost, VaultParameters, VaultState,
        },
    };

    // TODO: Ideally abstract those 2, so the tests dev doesnt has to keep
    // track of whats in the pool.
    pub const USDC_DENOM: &str = VAULT_CREATION_COST_DENOM;
    pub const OSMO_DENOM: &str = "uosmo";
    
    pub struct PoolMockup {
        pub pool_id: u64,
        pub initial_position_id: u64,
        pub app: OsmosisTestApp,
        pub deployer: SigningAccount,
        pub user1: SigningAccount,
        pub user2: SigningAccount,
        pub price: Decimal,
    }

    impl PoolMockup {
        pub fn new_with_spread(usdc_in: u128, osmo_in: u128, spread_factor: &str) -> Self {
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
                        tick_spacing: 30,
                        spread_factor: Decimal::from_str(spread_factor).unwrap().atomics().into()
                    }]
                },
                deployer.address(),
                &deployer,
            )
            .unwrap();

            // NOTE: Could fail if we test multiple pools/positions.
            let pool_id = 1;
            let initial_position_id = 1;

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
            app.increase_time(TWAP_SECONDS);

            let price = Decimal::new(osmo_in.into()) / Decimal::new(usdc_in.into());

            Self {
                pool_id, initial_position_id, app, deployer, user1, user2, price
            }
            
        }

        pub fn new(usdc_in: u128, osmo_in: u128) -> Self {
            Self::new_with_spread(usdc_in, osmo_in, "0.01")
        }

        pub fn swap_osmo_for_usdc(&self, from: &SigningAccount, osmo_in: u128) -> Result<Uint128> {
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

        pub fn swap_usdc_for_osmo(&self, from: &SigningAccount, usdc_in: u128) -> Result<Uint128> {
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

        pub fn osmo_balance_query(&self, address: &str) -> Uint128 {
            let bank = Bank::new(&self.app);
            let amount = bank.query_balance(&QueryBalanceRequest {
                address: address.into(),
                denom: OSMO_DENOM.into()
            }).unwrap().balance.unwrap().amount;
            Uint128::from_str(&amount).unwrap()
        }

        pub fn usdc_balance_query(&self, address: &str) -> Uint128 {
            let bank = Bank::new(&self.app);
            let amount = bank.query_balance(&QueryBalanceRequest {
                address: address.into(),
                denom: USDC_DENOM.into()
            }).unwrap().balance.unwrap().amount;
            Uint128::from_str(&amount).unwrap()
        }

        pub fn position_query(&self, position_id: u64) -> Result<FullPositionBreakdown> {
            let cl = ConcentratedLiquidity::new(&self.app);
            let pos = cl.query_position_by_id(&PositionByIdRequest { position_id })?;
            Ok(pos.position.expect("oops"))
        }

        pub fn position_liquidity(&self, position_id: u64) -> Result<Decimal> {
            let pos = self.position_query(position_id)?;
            let liq = pos.position
                .map(|x| Uint128::from_str(&x.liquidity))
                .expect("oops")
                .map(|x| Decimal::raw(x.u128()))?;
            Ok(liq)
        }
    }

    pub fn store_vaults_code(wasm: &Wasm<OsmosisTestApp>, deployer: &SigningAccount) -> u64 {
        let contract_bytecode = std::fs::read(
            "target/wasm32-unknown-unknown/release/magma_core.wasm"
        ).unwrap();

        wasm.store_code(&contract_bytecode, None, deployer)
            .unwrap()
            .data
            .code_id
    }

    pub fn vault_params(base: &str, limit: &str, full: &str) -> VaultParametersInstantiateMsg {
        VaultParametersInstantiateMsg {
            full_range_weight: Decimal::from_str(full).unwrap().atomics(),
            base_factor: Decimal::from_str(base).unwrap().atomics(),
            limit_factor: Decimal::from_str(limit).unwrap().atomics(),
        }
    }

    pub fn rebalancer_anyone(price_factor_before_rebalance: &str, seconds_before_rebalance: u32) -> VaultRebalancerInstantiateMsg {
        VaultRebalancerInstantiateMsg::Anyone { 
            price_factor_before_rebalance: Decimal::from_str(price_factor_before_rebalance).unwrap().atomics(),
            seconds_before_rebalance
        }
    }

    pub fn deposit_msg<T: ToString>(to: T) -> ExecuteMsg {
        ExecuteMsg::Deposit(DepositMsg { 
            amount0_min: Uint128::zero(),
            amount1_min: Uint128::zero(),
            to: to.to_string()
        })
    }

    pub struct VaultMockup<'a> {
        pub vault_addr: Addr,
        pub wasm: Wasm<'a, OsmosisTestApp>
    }

    impl VaultMockup<'_> {
        pub fn new(pool_info: &PoolMockup, params: VaultParametersInstantiateMsg) -> VaultMockup {
            Self::new_with_rebalancer(pool_info, params, VaultRebalancerInstantiateMsg::Admin {})
        }

        pub fn new_with_rebalancer(
            pool_info: &PoolMockup,
            params: VaultParametersInstantiateMsg,
            rebalancer: VaultRebalancerInstantiateMsg
        ) -> VaultMockup {
            let wasm = Wasm::new(&pool_info.app);
            let code_id = store_vaults_code(&wasm, &pool_info.deployer);
            let api = mock_dependencies().api;

            let usdc_fee = Coin::new(VaultCreationCost::default().0.into(), USDC_DENOM);
            let vault_addr = wasm
                .instantiate(
                    code_id,
                    &InstantiateMsg {
                        vault_info: VaultInfoInstantiateMsg {
                            pool_id: pool_info.pool_id,
                            vault_name: "My USDC/OSMO vault".into(),
                            vault_symbol: "USDCOSMOV".into(),
                            admin: Some(pool_info.deployer.address()),
                            admin_fee: ProtocolFee::default().0.0.atomics(),
                            rebalancer
                        },
                        vault_parameters: params,
                    },
                    None,
                    Some("my vault"),
                    &[usdc_fee],
                    &pool_info.deployer,
                )
                .unwrap()
                .data
                .address;

            let vault_addr = api.addr_validate(&vault_addr).unwrap();

            VaultMockup { vault_addr, wasm }

        }

        pub fn deposit(
            &self,
            usdc: u128,
            osmo: u128,
            from: &SigningAccount
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            let (amount0, amount1) = (usdc, osmo);

            let execute_msg = &deposit_msg(from.address());
            let coin0 = Coin::new(amount0, USDC_DENOM);
            let coin1 = Coin::new(amount1, OSMO_DENOM);

            if amount0 == 0 && amount1 == 0 {
                unimplemented!()
            } else if amount0 == 0 {
                Ok(self.wasm.execute(
                    self.vault_addr.as_ref(),
                    execute_msg,
                    &[coin1],
                    from
                )?)
            } else if amount1 == 0 {
                Ok(self.wasm.execute(
                    self.vault_addr.as_ref(),
                    execute_msg,
                    &[coin0],
                    from
                )?)
            } else {
                Ok(self.wasm.execute(
                    self.vault_addr.as_ref(),
                    execute_msg,
                    &[coin0, coin1],
                    from
                )?)
            }
        }

        pub fn rebalance(
            &self,
            from: &SigningAccount
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(), &ExecuteMsg::Rebalance {}, &[], from
            )?)
        }

        pub fn withdraw(
            &self,
            shares: Uint128,
            from: &SigningAccount
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::Withdraw(WithdrawMsg {
                    shares,
                    amount0_min: Uint128::zero(),
                    amount1_min: Uint128::zero(),
                    to: from.address()
                }),
                &[],
                from
            )?)
        }

        pub fn admin_withdraw(
            &self,
            from: &SigningAccount
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::WithdrawAdminFees {},
                &[],
                from
            )?)
        }

        pub fn protocol_withdraw(
            &self,
            from: &SigningAccount
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::WithdrawProtocolFees {},
                &[],
                from
            )?)
        }

        pub fn propose_new_admin(
            &self,
            from: &SigningAccount,
            new: Option<&SigningAccount>
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::ProposeNewAdmin { new_admin: new.map(|x| x.address()) },
                &[],
                from
            )?)
        }
        
        pub fn accept_new_admin(
            &self,
            from: &SigningAccount
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::AcceptNewAdmin {},
                &[],
                from
            )?)
        }

        pub fn burn_vault_admin(
            &self,
            from: &SigningAccount
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::BurnVaultAdmin {},
                &[],
                from
            )?)
        }

        pub fn change_vault_rebalancer(
            &self,
            from: &SigningAccount,
            new_rebalancer: VaultRebalancerInstantiateMsg
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::ChangeVaultRebalancer(new_rebalancer),
                &[],
                from
            )?)
        }

        pub fn change_vault_parameters(
            &self,
            from: &SigningAccount,
            new_paramerers: VaultParametersInstantiateMsg
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::ChangeVaultParameters(new_paramerers),
                &[],
                from
            )?)
        }

        pub fn change_admin_fee(
            &self,
            from: &SigningAccount,
            new_fee: &str
        ) -> Result<ExecuteResponse<MsgExecuteContractResponse>> {
            Ok(self.wasm.execute(
                self.vault_addr.as_ref(),
                &ExecuteMsg::ChangeAdminFee { new_admin_fee: Decimal::from_str(new_fee).unwrap().atomics() },
                &[],
                from
            )?)
        }

        pub fn vault_balances_query(&self) -> VaultBalancesResponse {
            self.wasm.query(
                self.vault_addr.as_ref(),
                &QueryMsg::VaultBalances { }
            ).unwrap()
        }

        pub fn position_balances_query(&self, position_type: PositionType) -> PositionBalancesWithFeesResponse {
            self.wasm.query(
                self.vault_addr.as_ref(),
                &QueryMsg::PositionBalancesWithFees { position_type },
            ).unwrap()
        }

        pub fn token_info_query(&self) -> TokenInfo {
            self.wasm.query(
                self.vault_addr.as_ref(),
                &QueryMsg::TokenInfo {  }
            ).unwrap()
        }

        pub fn shares_query(&self, address: &str) -> Uint128 {
            let res: cw20::BalanceResponse = self.wasm.query(
                self.vault_addr.as_ref(),
                &QueryMsg::Balance { address: address.into() }
            ).unwrap();
            res.balance
        }

        pub fn vault_state_query(&self) -> VaultState {
            self.wasm.query(
                self.vault_addr.as_ref(),
                &QueryMsg::VaultState {}
            ).unwrap()
        }

        pub fn vault_parameters_query(&self) -> VaultParameters {
            self.wasm.query(
                self.vault_addr.as_ref(),
                &QueryMsg::VaultParameters {}
            ).unwrap()
        }

        pub fn vault_fees_query(&self) -> FeesInfo {
            self.wasm.query(
                self.vault_addr.as_ref(),
                &QueryMsg::FeesInfo {}
            ).unwrap()
        }
    }
}
