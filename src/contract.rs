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
    use std::{rc::Rc, str::FromStr};

    use crate::{constants::{MAX_TICK, MIN_TICK}, msg::{DepositMsg, VaultBalancesResponse, VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg, WithdrawMsg}, utils::price_function_inv};

    use super::*;
    use cosmwasm_std::{coin, testing::mock_dependencies, Addr, Api, Coin, Decimal};
    use cw20::BalanceResponse;
    use osmosis_std::types::{cosmwasm::wasm::v1::MsgExecuteContractResponse, osmosis::{
        concentratedliquidity::v1beta1::{
            CreateConcentratedLiquidityPoolsProposal, MsgCreatePosition, PoolRecord, PositionByIdRequest
        }, poolmanager::v1beta1::{MsgSwapExactAmountIn, SwapAmountInRoute}}}
    ;
    use osmosis_test_tube::{Account, ConcentratedLiquidity, ExecuteResponse, GovWithAppAccess, Module, OsmosisTestApp, PoolManager, SigningAccount, Wasm};

    struct PoolMockupInfo {
        pool_id: u64,
        app: OsmosisTestApp,
        deployer: SigningAccount,
        _price: Decimal,
    }

    const USDC_DENOM: &str = "ibc/DE6792CF9E521F6AD6E9A4BDF6225C9571A3B74ACC0A529F92BC5122A39D2E58";
    const OSMO_DENOM: &str = "uosmo";

    fn create_basic_usdc_osmo_pool(x_bal: u128, y_bal: u128) -> Box<PoolMockupInfo> {
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
                    spread_factor: Decimal::from_str("0.01").unwrap().atomics().into()
                }]
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

        Box::new(PoolMockupInfo {
            pool_id,
            app,
            deployer,
            _price: Decimal::new(y_bal.into()) / Decimal::new(x_bal.into()),
        })
    }

    fn store_vaults_code(wasm: &Wasm<OsmosisTestApp>, deployer: &SigningAccount) -> u64 {
        let contract_bytecode =
            std::fs::read("target/wasm32-unknown-unknown/release/magma_core.wasm").unwrap();

        wasm.store_code(&contract_bytecode, None, deployer)
            .unwrap()
            .data
            .code_id
    }

    type ExeRes = anyhow::Result<ExecuteResponse<MsgExecuteContractResponse>>;
    struct InstVaultRes<'a> {
        vault_addr: Addr,
        wasm: Rc<Wasm<'a, OsmosisTestApp>>,
        deposit: Box<dyn Fn(u128, u128, &SigningAccount) -> ExeRes + 'a>,
        rebalance: Box<dyn Fn(&SigningAccount) -> ExeRes + 'a>
    }

    fn inst_vault<'a>(
        pool_info: &'a PoolMockupInfo,
        params: VaultParametersInstantiateMsg,
    ) -> InstVaultRes {
        let wasm = Rc::new(Wasm::new(&pool_info.app));
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

        let deposit_vault_addr = vault_addr.clone();
        let deposit_wasm = Rc::clone(&wasm);
        let deposit = move |amount0, amount1, from: &SigningAccount| {
            let execute_msg = &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(amount0),
                amount1: Uint128::new(amount1),
                amount0_min: Uint128::new(amount0),
                amount1_min: Uint128::new(amount1),
                to: from.address()
            });

            let coin0 = Coin::new(amount0, USDC_DENOM);
            let coin1 = Coin::new(amount1, OSMO_DENOM);

            if amount0 == 0 && amount1 == 0 {
                unimplemented!()
            } else if amount0 == 0 {
                Ok(deposit_wasm.execute(
                    &deposit_vault_addr.to_string(),
                    execute_msg,
                    &[coin1],
                    from
                )?)
            } else if amount1 == 0 {
                Ok(deposit_wasm.execute(
                    &deposit_vault_addr.to_string(),
                    execute_msg,
                    &[coin0],
                    from
                )?)
            } else {
                Ok(deposit_wasm.execute(
                    &deposit_vault_addr.to_string(),
                    execute_msg,
                    &[coin0, coin1],
                    from
                )?)
            }
        };

        let rebalance_vault_addr = vault_addr.clone();
        let rebalance_wasm = Rc::clone(&wasm);
        let rebalance = move |from: &SigningAccount| -> anyhow::Result<_> {
            Ok(rebalance_wasm.execute(
                &rebalance_vault_addr.to_string(),
                &ExecuteMsg::Rebalance {},
                &[],
                from
            )?)
        };

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

        InstVaultRes {
            vault_addr: api.addr_validate(&vault_addr).unwrap(),
            wasm,
            deposit: Box::new(deposit),
            rebalance: Box::new(rebalance)
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
    fn normal_rebalance() {
        let pool_info = create_basic_usdc_osmo_pool(100_000, 200_000);
        let vault_inst = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: "2".into(),
                limit_factor: "1.45".into(),
                full_range_weight: "0.55".into(),
            },
        );

        (vault_inst.deposit)(1_000, 1_500, &pool_info.deployer).unwrap();
        (vault_inst.rebalance)(&pool_info.deployer).unwrap();
    }

    #[test]
    fn normal_rebalance_dual() {
        let pool_info = create_basic_usdc_osmo_pool(100_000, 200_000);
        let vault_inst = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: "2".into(),
                limit_factor: "1.45".into(),
                full_range_weight: "0.55".into(),
            },
        );

        (vault_inst.deposit)(1_500, 1_000, &pool_info.deployer).unwrap();
        (vault_inst.rebalance)(&pool_info.deployer).unwrap();
    }

    #[test]
    fn rebalance_in_proportion() {
        let pool_balance0 = 100_000;
        let pool_balance1 = 200_000;
        let pool_info = create_basic_usdc_osmo_pool(pool_balance0, pool_balance1);

        let vault_inst = inst_vault(
            &pool_info,
            VaultParametersInstantiateMsg {
                base_factor: "2".into(),
                limit_factor: "1.45".into(),
                full_range_weight: "0.55".into(),
            },
        );
    
        (vault_inst.deposit)(pool_balance0/2, pool_balance1/2, &pool_info.deployer).unwrap();
        (vault_inst.rebalance)(&pool_info.deployer).unwrap();
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
            &vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(42),
                amount1: Uint128::new(0),
                amount0_min: Uint128::new(42),
                amount1_min: Uint128::new(0),
                to: pool_info.deployer.address(),
            }),
            &[Coin::new(42, USDC_DENOM)],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.to_string(),
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
            &vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(0),
                amount1: Uint128::new(42),
                amount0_min: Uint128::new(0),
                amount1_min: Uint128::new(42),
                to: pool_info.deployer.address(),
            }),
            &[Coin::new(42, OSMO_DENOM)],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.to_string(),
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
            &vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
                amount0_min: Uint128::new(vault_x),
                amount1_min: Uint128::new(vault_y),
                to: pool_info.deployer.address(),
            }),
            &[
                Coin::new(vault_x, USDC_DENOM),
                Coin::new(vault_y, OSMO_DENOM),
            ],
            &pool_info.deployer,
        )
        .unwrap();

        wasm.execute(
            &vault_addr.to_string(),
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
            &vault_addr.to_string(),
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
            &vault_addr.to_string(),
            &ExecuteMsg::Rebalance {},
            &[],
            &pool_info.deployer,
        )
        .unwrap();
    }

    #[test]
    fn deposit_withdrawal_rebalance_smoke_test() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_info = create_basic_usdc_osmo_pool(pool_x, pool_y);
        let (vault_addr, wasm) = inst_vault(&pool_info, VaultParametersInstantiateMsg { 
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into()
        });

        let (vault_x, vault_y) = (1_000, 1_500);
        wasm.execute(
            &vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
                amount0_min: Uint128::new(vault_x),
                amount1_min: Uint128::new(vault_y),
                to: pool_info.deployer.address()
            }),
            &[
                Coin::new(vault_x, USDC_DENOM),
                Coin::new(vault_y, OSMO_DENOM)
            ],
            &pool_info.deployer
        ).unwrap();

        let shares_got: BalanceResponse = wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::Balance { 
                address: pool_info.deployer.address() 
            }
        ).unwrap();

        wasm.execute(&vault_addr.to_string(), &ExecuteMsg::Rebalance {}, &[], &pool_info.deployer)
            .unwrap();

        let pm = PoolManager::new(&pool_info.app);
        pm.swap_exact_amount_in(
            MsgSwapExactAmountIn {
                sender: pool_info.deployer.address(),
                routes: vec![SwapAmountInRoute {
                    pool_id: pool_info.pool_id, token_out_denom: USDC_DENOM.into()
                }],
                token_in: Some(Coin::new(pool_y/2, OSMO_DENOM).into()),
                token_out_min_amount: "1".into()
            }, &pool_info.deployer
        ).unwrap();

        let state: VaultState = wasm.query(&vault_addr.to_string(), &QueryMsg::VaultPositions { }).unwrap();
        println!("{:?}", state);

        let cl = ConcentratedLiquidity::new(&pool_info.app);
        let getpos = |id: Option<u64>| {
            id.map(|position_id| cl.query_position_by_id(&PositionByIdRequest { position_id }))
                .map(|x| x.unwrap().position.unwrap())
        };

        println!("{:?}", getpos(state.full_range_position_id).map(|p| p.claimable_spread_rewards));
        println!("{:?}", getpos(state.base_position_id).map(|p| p.claimable_spread_rewards));
        println!("{:?}", getpos(state.limit_position_id).map(|p| p.claimable_spread_rewards));

        let shares_got = shares_got.balance;

        wasm.execute(
            &vault_addr.to_string(), 
            &ExecuteMsg::Withdraw(
                WithdrawMsg {
                    shares: shares_got,
                    amount0_min: Uint128::zero(),
                    amount1_min: Uint128::zero(),
                    to: pool_info.deployer.address()
                }
            ),
            &[],
            &pool_info.deployer
        ).unwrap();
    }

    #[test]
    fn withdraw_without_rebalances() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_info = create_basic_usdc_osmo_pool(pool_x, pool_y);
        let (vault_addr, wasm) = inst_vault(&pool_info, VaultParametersInstantiateMsg { 
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into()
        });

        let (vault_x, vault_y) = (1_000, 1_500);
        wasm.execute(
            &vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
                amount0_min: Uint128::new(vault_x),
                amount1_min: Uint128::new(vault_y),
                to: pool_info.deployer.address()
            }),
            &[
                Coin::new(vault_x, USDC_DENOM),
                Coin::new(vault_y, OSMO_DENOM)
            ],
            &pool_info.deployer
        ).unwrap();

        let vault_balances_before_withdrawal: VaultBalancesResponse= wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::VaultBalances { }
        ).unwrap();

        println!("{:?}", vault_balances_before_withdrawal);

        let shares_got: BalanceResponse = wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::Balance { 
                address: pool_info.deployer.address() 
            }
        ).unwrap();

        wasm.execute(
            &vault_addr.to_string(), 
            &ExecuteMsg::Withdraw(
                WithdrawMsg {
                    shares: shares_got.balance,
                    amount0_min: Uint128::zero(),
                    amount1_min: Uint128::zero(),
                    to: pool_info.deployer.address()
                }
            ),
            &[],
            &pool_info.deployer
        ).unwrap();

        let vault_balances_after_withdrawal: VaultBalancesResponse= wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::VaultBalances { }
        ).unwrap();
        
        println!("{:?}", vault_balances_after_withdrawal);
    }

    #[test]
    fn withdraw_limit_without_rebalances() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_info = create_basic_usdc_osmo_pool(pool_x, pool_y);
        let (vault_addr, wasm) = inst_vault(&pool_info, VaultParametersInstantiateMsg { 
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into()
        });

        let (vault_x, vault_y) = (0, 6969);
        wasm.execute(
            &vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
                amount0_min: Uint128::new(vault_x),
                amount1_min: Uint128::new(vault_y),
                to: pool_info.deployer.address()
            }),
            &[
                coin(vault_y, OSMO_DENOM)
            ],
            &pool_info.deployer
        ).unwrap();

        let vault_balances_before_withdrawal: VaultBalancesResponse= wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::VaultBalances { }
        ).unwrap();

        println!("{:?}", vault_balances_before_withdrawal);

        let shares_got: BalanceResponse = wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::Balance { 
                address: pool_info.deployer.address() 
            }
        ).unwrap();

        wasm.execute(
            &vault_addr.to_string(), 
            &ExecuteMsg::Withdraw(
                WithdrawMsg {
                    shares: shares_got.balance,
                    amount0_min: Uint128::zero(),
                    amount1_min: Uint128::zero(),
                    to: pool_info.deployer.address()
                }
            ),
            &[],
            &pool_info.deployer
        ).unwrap();

        let vault_balances_after_withdrawal: VaultBalancesResponse= wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::VaultBalances { }
        ).unwrap();
        
        println!("{:?}", vault_balances_after_withdrawal);
    }

    #[test]
    fn withdraw_with_min_amounts() {
        let (pool_x, pool_y) = (100_000, 200_000);
        let pool_info = create_basic_usdc_osmo_pool(pool_x, pool_y);
        let (vault_addr, wasm) = inst_vault(&pool_info, VaultParametersInstantiateMsg { 
            base_factor: "2".into(),
            limit_factor: "1.45".into(),
            full_range_weight: "0.55".into()
        });

        let (vault_x, vault_y) = (1_000, 1_500);
        wasm.execute(
            &vault_addr.to_string(),
            &ExecuteMsg::Deposit(DepositMsg {
                amount0: Uint128::new(vault_x),
                amount1: Uint128::new(vault_y),
                amount0_min: Uint128::new(vault_x),
                amount1_min: Uint128::new(vault_y),
                to: pool_info.deployer.address()
            }),
            &[
                coin(vault_x, USDC_DENOM),
                coin(vault_y, OSMO_DENOM)
            ],
            &pool_info.deployer
        ).unwrap();

        let vault_balances_before_withdrawal: VaultBalancesResponse= wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::VaultBalances { }
        ).unwrap();

        let shares_got: BalanceResponse = wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::Balance { 
                address: pool_info.deployer.address() 
            }
        ).unwrap();

        let improper_withdrawal = wasm.execute(
            &vault_addr.to_string(), 
            &ExecuteMsg::Withdraw(
                WithdrawMsg {
                    shares: shares_got.balance,
                    amount0_min: vault_balances_before_withdrawal.bal0 + Uint128::one(),
                    amount1_min: vault_balances_before_withdrawal.bal1 + Uint128::one(),
                    to: pool_info.deployer.address()
                }
            ),
            &[],
            &pool_info.deployer
        );
        assert!(improper_withdrawal.is_err());

        wasm.execute(
            &vault_addr.to_string(), 
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

        let vault_balances_after_withdrawal: VaultBalancesResponse= wasm.query(
            &vault_addr.to_string(),
            &QueryMsg::VaultBalances { }
        ).unwrap();

        assert!(vault_balances_after_withdrawal.bal0.is_zero());
        assert!(vault_balances_after_withdrawal.bal1.is_zero());
        assert!(vault_balances_after_withdrawal.protocol_unclaimed_fees0.is_zero());
        assert!(vault_balances_after_withdrawal.protocol_unclaimed_fees1.is_zero());
    }
}
