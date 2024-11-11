use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, Uint128};
use cw20::{BalanceResponse, Expiration, TokenInfoResponse};
use crate::state::{FeesInfo, PositionType, VaultInfo, VaultState};

#[cw_serde]
pub struct VaultParametersInstantiateMsg {
    /// 18 decimal places [`PriceFactor`].
    pub base_factor: Uint128,
    /// 18 decimal places [`PriceFactor`].
    pub limit_factor: Uint128,
    /// 18 decimal places [`Weight`].
    pub full_range_weight: Uint128,
}

#[cw_serde]
pub struct VaultInfoInstantiateMsg {
    pub pool_id: u64,
    pub vault_name: String,
    pub vault_symbol: String,
    pub admin: Option<String>,
    /// 18 decimal places [`Weight`].
    pub admin_fee: Uint128,
    pub rebalancer: VaultRebalancerInstantiateMsg,
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
        /// 18 decimal places [`PriceFactor`]. Anyone will only be able to 
        /// rebalance if the price has moved this factor since the last rebalance.
        price_factor_before_rebalance: Uint128,
        /// Anyone can only rebalance if this time has passed since the last rebalace.
        seconds_before_rebalance: u32
    }
}

#[cw_serde]
pub struct InstantiateMsg {
    pub vault_info: VaultInfoInstantiateMsg,
    pub vault_parameters: VaultParametersInstantiateMsg
}

#[cw_serde]
pub struct DepositMsg {
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
    // Core Logic.
    Deposit(DepositMsg),
    Rebalance {},
    Withdraw(WithdrawMsg),

    // Admin/Protocol operations.
    WithdrawProtocolFees {},
    WithdrawAdminFees {},
    ProposeNewAdmin { new_admin: Option<String> },
    AcceptNewAdmin {},
    BurnVaultAdmin {},
    ChangeVaultRebalancer(VaultRebalancerInstantiateMsg),
    ChangeVaultParameters(VaultParametersInstantiateMsg),
    ChangeAdminFee { new_admin_fee: Uint128 },
    ChangeProtocolFee { new_protocol_fee: Uint128 },

    // Cw20 Realization.
    Transfer { recipient: String, amount: Uint128 },
    Burn { amount: Uint128 },
    Send { contract: String, amount: Uint128, msg: Binary },
    IncreaseAllowance { spender: String, amount: Uint128, expires: Option<Expiration> },
    DecreaseAllowance { spender: String, amount: Uint128, expires: Option<Expiration> },
    TransferFrom { owner: String, recipient: String, amount: Uint128 },
    SendFrom { owner: String, contract: String, amount: Uint128, msg: Binary },
    BurnFrom { owner: String, amount: Uint128 },
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
    VaultInfo {},
    #[returns(FeesInfo)]
    FeesInfo {}
}

#[cw_serde]
pub struct VaultBalancesResponse {
    /// All of token0 the vault has access to, without counting protocol/admin fees.
    pub bal0: Uint128,
    /// All of token1 the vault has access to, without counting protocol/admin fees.
    pub bal1: Uint128,
    pub protocol_unclaimed_fees0: Uint128,
    pub protocol_unclaimed_fees1: Uint128,
    pub admin_unclaimed_fees0: Uint128,
    pub admin_unclaimed_fees1: Uint128,
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

