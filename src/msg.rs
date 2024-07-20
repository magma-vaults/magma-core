use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Deps, StdResult};

use crate::state::{PositiveTick, TickSpacing, VaultInfo, VaultManagementConfig, VaultParametersConfig, VaultRebalancer, VaultState, Weight};

#[cw_serde]
pub struct VaultParametersInstantiateMsg {
    pub base_threshold: u32,
    pub limit_threshold: u32,
    pub full_range_weight: u32 
}

#[cw_serde]
pub struct VaultInfoInstantaiteMsg {
    pub pool_id: u64,
    pub denom0: String,
    pub denom1: String,
    pub tick_spacing: u32,
    pub vault_name: String,
    pub vault_symbol: String
}

#[cw_serde]
pub enum VaultRebalancerInstantiateMsg {
    /// Only the contract admin can trigger rebalances.
    Admin {},
    /// Any delegated address decided by the admin can trigger rebalances.
    Delegate { rebalancer: String },
    /// Anyone can trigger rebalances, its the only option if there isnt a
    /// vault manager.
    Anyone {},
}

#[cw_serde]
pub struct VaultManagementConfigInstantiateMsg {
    pub admin: Option<String>,
    pub rebalancer: VaultRebalancerInstantiateMsg,
}

#[cw_serde]
pub struct InstantiateMsg {
    pub vault_info: VaultInfoInstantaiteMsg,
    pub vault_management_config: VaultManagementConfigInstantiateMsg,
    pub vault_parameters: VaultParametersInstantiateMsg
}

impl VaultRebalancerInstantiateMsg {
    pub fn addr_validate(self, deps: Deps) -> StdResult<VaultRebalancer> {
        use VaultRebalancer::*;
        match self {
            Self::Delegate { rebalancer: x } => Ok(Delegate {rebalancer: deps.api.addr_validate(&x)?}),
            Self::Admin {} => Ok(Admin {}),
            Self::Anyone {} => Ok(Anyone {}),
        }
    }
}

impl InstantiateMsg {
    // TODO Should return a Result with reasons.
    pub fn validate(self, deps: Deps) -> Option<VaultState> {
        
        let vault_info = VaultInfo {
            pool_id: self.vault_info.pool_id,
            denom0: self.vault_info.denom0,
            denom1: self.vault_info.denom1,
            tick_spacing: TickSpacing::new(self.vault_info.tick_spacing)?,
            vault_name: self.vault_info.vault_name,
            vault_symbol: self.vault_info.vault_symbol
        };

        let vault_management_config = {
            let management = self.vault_management_config;

            let admin = if let Some(addr) = &management.admin {
                Some(deps.api.addr_validate(addr).ok()?)
            } else { 
                None 
            };

            VaultManagementConfig {
                admin,
                rebalancer: management.rebalancer.addr_validate(deps).ok()?
            }
        };

        let vault_parameters = {

            let VaultParametersInstantiateMsg {
                base_threshold, limit_threshold, full_range_weight
            } = self.vault_parameters;

            let tick_spacing = vault_info.tick_spacing.clone();

            VaultParametersConfig {
                base_threshold: PositiveTick::new(base_threshold, tick_spacing.clone())?,
                limit_threshold: PositiveTick::new(limit_threshold, tick_spacing)?,
                full_range_weight: Weight::new(full_range_weight)?
            }
        };

        Some(VaultState {
            vault_info, vault_management_config, vault_parameters
        })
    }
}


#[cw_serde]
pub struct DepositMsg {
    pub amount0: u128,
    pub amount1: u128,
    pub amount0_min: u128,
    pub amount1_min: u128,
    pub to: String
}

#[cw_serde]
pub enum ExecuteMsg {
    Deposit(DepositMsg)
}

