use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::Item;

use crate::{error::ContractError, msg::InstantiateMsg};

/// TODO 
#[cw_serde]
pub struct VaultParametersConfig {
}

#[cw_serde]
pub enum VaultManager {
    /// If `None`, then the Vault is immutable and you cant change its parameters.
    None {},
    /// In any other case, the contract admin will be able to reconfigure the vault.
    Admin {},
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
    pub manager: VaultManager,
    pub rebalancer: VaultRebalancer,
    pub config: VaultParametersConfig,
}

pub const VAULT_STATE: Item<VaultState> = Item::new("vault_state");


