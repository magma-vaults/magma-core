use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Deps, StdResult};

use crate::state::{VaultManager, VaultParametersConfig, VaultRebalancer};

#[cw_serde]
pub struct TokenInfo {
}

#[cw_serde]
pub enum VaultRebalancerInstantiationMsg {
    /// Only the contract admin can trigger rebalances.
    Admin {},
    /// Any delegated address decided by the admin can trigger rebalances.
    Delegate { rebalancer: String },
    /// Anyone can trigger rebalances, its the only option if there isnt a
    /// vault manager.
    Anyone {},
}

#[cw_serde]
pub struct InstantiateMsg {
    pub pool: String,
    pub manager: VaultManager,
    pub rebalancer: VaultRebalancerInstantiationMsg,
    pub config: VaultParametersConfig,
}

impl VaultRebalancerInstantiationMsg {
    pub fn addr_validate(self, deps: Deps) -> StdResult<VaultRebalancer> {
        use VaultRebalancer::*;
        match self {
            Self::Delegate { rebalancer } => Ok(Delegate { rebalancer: deps.api.addr_validate(&rebalancer)? }),
            Self::Admin {} => Ok(Admin {}),
            Self::Anyone {} => Ok(Anyone {}),
        }
    }
}
