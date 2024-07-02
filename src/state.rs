use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::Item;

/// 6 decimal point precision weight, represented internally as an `u32`.
#[cw_serde]
pub struct Weight(u32);
impl Weight {
    pub fn new(value: u32) -> Option<Self> {
        if value > u32::pow(10, 6) { None } else { Some(Self(value)) }
    }
}

#[cw_serde]
pub struct TickSpacing(u32);
impl TickSpacing {
    pub fn new(value: u32) -> Option<Self>{
        if value == 0 { None } else { Some(Self(value)) }
    }
}

#[cw_serde]
pub struct PositiveTick(u32);
impl PositiveTick {
    pub fn new(value: u32, TickSpacing(spacing): TickSpacing) -> Option<Self> {
        if value == 0 || value % spacing != 0 { None }
        else { Some(Self(value)) }
    }
}


#[cw_serde]
pub struct VaultParametersConfig {
    pub base_threshold: PositiveTick,
    pub limit_threshold: PositiveTick,
    pub full_range_weight: Weight
}

#[cw_serde]
pub struct VaultInfo {
    pub pool_id: u64,
    pub denom0: String,
    pub denom1: String,
    pub tick_spacing: TickSpacing,
    pub vault_name: String,
    pub vault_symbol: String
}

#[cw_serde]
pub enum VaultRebalancer {
    /// Only the contract admin can trigger rebalances
    Admin {},
    /// Any delegated address decided by the admin can trigger rebalances.
    Delegate { rebalancer: Addr },
    /// Anyone can trigger rebalances, its the only option if the vault
    /// doesnt has an admin.
    Anyone {},
}

#[cw_serde]
pub struct VaultManagementConfig {
    pub admin: Option<Addr>,
    pub rebalancer: VaultRebalancer,
}

#[cw_serde]
pub struct VaultState {
    pub vault_info: VaultInfo,
    pub vault_management_config: VaultManagementConfig,
    pub vault_parameters: VaultParametersConfig
}

pub const VAULT_STATE: Item<VaultState> = Item::new("vault_state");

