use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::Item;


/// TODO 
#[cw_serde]
pub struct VaultParametersConfig {
}

#[cw_serde]
pub enum VaultRebalancer {
    /// Only the contract admin can trigger rebalances.
    Admin {},
    /// Any delegated address decided by the admin can trigger rebalances.
    Delegate { rebalancer: Addr },
    /// Anyone can trigger rebalances, its the only option if there isnt a
    /// vault manager.
    Anyone {},
}

#[cw_serde]
pub struct VaultState {
    pub pool: Addr,
    pub admin: Option<Addr>,
    pub rebalancer: VaultRebalancer,
    pub config: VaultParametersConfig,
}

pub const VAULT_STATE: Item<VaultState> = Item::new("vault_state");


