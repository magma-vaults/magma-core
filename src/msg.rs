use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Uint128;
use cw20::{BalanceResponse, TokenInfoResponse};
use crate::state::{PositionType, VaultInfo, VaultState};

#[cw_serde]
pub struct VaultParametersInstantiateMsg {
    pub base_factor: String, // Decimal value, greater or equal to 1.
    pub limit_factor: String, // Decimal value, greater or equal to 1.
    pub full_range_weight: String // Decimal value, in range [0, 1].
}

#[cw_serde]
pub struct VaultInfoInstantiateMsg {
    pub pool_id: u64,
    pub vault_name: String,
    pub vault_symbol: String,
    pub admin: Option<String>,
    pub rebalancer: VaultRebalancerInstantiateMsg
}

#[cw_serde]
pub enum VaultRebalancerInstantiateMsg {
    /// Only the contract admin can trigger rebalances.
    Admin {},
    /// Any delegated address decided by the admin can trigger rebalances.
    Delegate { rebalancer: String },
    /// Anyone can trigger rebalances, its the only option if the vault
    /// doesnt has an admin. In that case, the specified parameters will
    /// determine if a rebalance is possible.
    Anyone {
        /// Decimal value, greater or equal than 1. Anyone will only be able to
        /// rebalance if the price has moved this factor since the last rebalance.
        price_factor_before_rebalance: String,
        /// Anyone can only rebalance if this time has passed since the last rebalace.
        seconds_before_rabalance: u64
    }
}

#[cw_serde]
pub struct InstantiateMsg {
    pub vault_info: VaultInfoInstantiateMsg,
    pub vault_parameters: VaultParametersInstantiateMsg
}

#[cw_serde]
pub struct DepositMsg {
    pub amount0: Uint128,
    pub amount1: Uint128,
    pub amount0_min: Uint128,
    pub amount1_min: Uint128,
    pub to: String // Addr to mint shares to.
}

#[cw_serde]
pub struct WithdrawMsg {
    pub shares: Uint128,
    pub amount0_min: Uint128,
    pub amount1_min: Uint128,
    pub to: String
}

#[cw_serde]
pub enum ExecuteMsg {
    Deposit(DepositMsg),
    Rebalance {},
    Withdraw(WithdrawMsg)
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// All value held by the vault, including balances in the contract, 
    /// balances in positions, and uncollected fees.
    #[returns(VaultBalancesResponse)]
    VaultBalances {},
    #[returns(PositionBalancesWithFeesResponse)]
    PositionBalancesWithFees { position_type: PositionType },
    #[returns(CalcSharesAndUsableAmountsResponse)]
    CalcSharesAndUsableAmounts { for_amount0: Uint128, for_amount1: Uint128 },
    #[returns(BalanceResponse)]
    Balance { address: String },
    #[returns(VaultState)]
    VaultState {},
    #[returns(TokenInfoResponse)]
    TokenInfo {},
    #[returns(VaultInfo)]
    VaultInfo {}
}

#[cw_serde]
pub struct VaultBalancesResponse {
    /// All of token0 held by the vault, but without counting the protocol fees.
    pub bal0: Uint128,
    /// All of token1 held by the vault, but without counting the protocol fees.
    pub bal1: Uint128,
    pub protocol_unclaimed_fees0: Uint128,
    pub protocol_unclaimed_fees1: Uint128
}

#[cw_serde]
#[derive(Default)]
pub struct PositionBalancesWithFeesResponse {
    pub bal0: Uint128,
    pub bal1: Uint128,
    pub bal0_fees: Uint128,
    pub bal1_fees: Uint128,
}

#[cw_serde]
#[derive(Default)]
pub struct CalcSharesAndUsableAmountsResponse {
    pub shares: Uint128,
    pub usable_amount0: Uint128,
    pub usable_amount1: Uint128
}

