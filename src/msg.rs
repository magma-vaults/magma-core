use cosmwasm_schema::cw_serde;

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
    /// Anyone can trigger rebalances, its the only option if there isnt a
    /// vault manager.
    Anyone {}
}

#[cw_serde]
pub struct InstantiateMsg {
    pub vault_info: VaultInfoInstantiateMsg,
    pub vault_parameters: VaultParametersInstantiateMsg
}

#[cw_serde]
pub struct DepositMsg {
    pub amount0: String, // Decimal compatible value.
    pub amount1: String, // Decimal compatible value.
    pub amount0_min: String, // Decimal compatible value.
    pub amount1_min: String, // Decimal compatible value.
    pub to: String
}

#[cw_serde]
pub enum ExecuteMsg {
    Deposit(DepositMsg),
    Rebalance {}
}

