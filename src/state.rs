use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Deps, QuerierWrapper, Uint128};
use cw_storage_plus::Item;
use osmosis_std::types::osmosis::{
    concentratedliquidity::v1beta1::{FullTick, MsgCreatePosition, Pool, TickInfo},
    poolmanager::v1beta1::PoolmanagerQuerier
};

use readonly;

use crate::{constants::MIN_TICK, error::ContractError, msg::{VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg}};
use crate::constants::MAX_TICK;
use std::cmp::min_by_key;

#[cw_serde] #[readonly::make]
pub struct Weight(pub Decimal);
impl Weight {
    const MAX: Decimal = Decimal::one();

    pub fn new(value: u128) -> Option<Self> {
        let value = Decimal::new(Uint128::new(value));
        (value <= Self::MAX).then_some(Self(value))
    }
}

#[cw_serde] #[readonly::make]
pub struct PriceFactor(pub Decimal);
impl PriceFactor {
    pub fn new(value: u128) -> Option<Self> {
        let value = Decimal::new(Uint128::new(value));
        (value >= Decimal::one()).then_some(Self(value))
    }

    pub fn is_one(&self) -> bool { self.0 == Decimal::one() }
}

#[cw_serde] #[readonly::make]
pub struct PoolId(pub u64);
impl PoolId {
    pub fn new(pool_id: u64, querier: &QuerierWrapper) -> Option<Self> {
        let querier = PoolmanagerQuerier::new(querier);
        let encoded_pool = querier.pool(pool_id).ok()?.pool?;
        // The pool could only not be deserialized if `pool_id`
        // does not refer to a valid concentrated liquidity pool.
        Pool::try_from(encoded_pool).ok().and(Some(Self(pool_id)))
    }

    pub fn to_pool(&self, querier: &QuerierWrapper) -> Pool {
        let querier = PoolmanagerQuerier::new(querier);
        // Invariant: We already verified that `id` refers to a valid pool.
        querier.pool(self.0).unwrap().pool.unwrap().try_into().unwrap()
    }

    fn tick_spacing(&self, querier: &QuerierWrapper) -> i64 {
        // Invariant: Wont overflow under reasonable conditions.
        self.to_pool(querier).tick_spacing as i64
    }
}

#[cw_serde]
pub struct VaultParameters {
    // Price factor for the base order. Thus, if the current price is `p`,
    // then the base position will have range `[p/base_factor, p*base_factor]`.
    pub base_factor: PriceFactor,
    // Price factor for the limit order. Thus, if the current price is `p`,
    // then the limit position will have either range `[p/limit_factor, p]` or
    // `[p, p*limit_factor]`.
    pub limit_factor: PriceFactor,
    // cosmwasm_std::Decimal weight, zero if we dont want a full range position.
    pub full_range_weight: Weight,
    // TODO Put this into a separate struct for state. Those parameters above
    // should always be present after instantiation (INVARIANT).
    //
    // Position Ids are optional because: 
    // 1. Positions are oly created on rebalances.
    // 2. If any of the params is 0, then the position id for them might be None.
    pub base_position_id: Option<u64>,
    pub limit_position_id: Option<u64>,
    pub full_range_position_id: Option<u64>
}

impl VaultParameters {
    pub fn new(
        params: VaultParametersInstantiateMsg,
        vault_info: VaultInfo,
        querier: &QuerierWrapper
    ) -> Result<Self, ContractError> {

        let base_factor = PriceFactor::new(params.base_factor)
            .ok_or(ContractError::InvalidConfig {})?;
        
        let limit_factor = PriceFactor::new(params.limit_factor)
            .ok_or(ContractError::InvalidConfig {})?;

        let full_range_weight = Weight::new(params.full_range_weight)
            .ok_or(ContractError::InvalidConfig {})?;

        if base_factor.is_one() && limit_factor.is_one() {
            if full_range_weight.0 != Weight::MAX {
                return Err(ContractError::InvalidConfig {})
            }
        }

        Ok(VaultParameters {
            base_factor, limit_factor, full_range_weight,
            base_position_id: None, limit_position_id: None, full_range_position_id: None
        })
    }
}

#[cw_serde] #[readonly::make]
pub struct VaultInfo {
    #[readonly] pub pool_id: PoolId,
    #[readonly] pub vault_name: String,
    #[readonly] pub vault_symbol: String,
    pub admin: Option<Addr>,
    pub rebalancer: VaultRebalancer,
}

impl VaultInfo {
    pub fn new(info: VaultInfoInstantiateMsg, deps: Deps) -> Result<Self, ContractError> {
        let pool_id = PoolId::new(info.pool_id, &deps.querier)
            .ok_or(ContractError::InvalidConfig {})?;

        assert!(pool_id.0 == info.pool_id);

        let rebalancer = VaultRebalancer::new(info.rebalancer, deps)?;

        let admin = if let Some(admin) = info.admin {
            Some(deps.api.addr_validate(&admin)?)
        } else { 
            match rebalancer {
                VaultRebalancer::Anyone {} => Ok(None),
                _ => Err(ContractError::InvalidConfig {})
            }?
        };

        Ok(VaultInfo {
            pool_id, rebalancer, admin,
            vault_name: info.vault_name, vault_symbol: info.vault_symbol
        })
    }

    pub fn demon0(&self, querier: &QuerierWrapper) -> String {
        self.pool_id.to_pool(querier).token0
    }

    pub fn demon1(&self, querier: &QuerierWrapper) -> String {
        self.pool_id.to_pool(querier).token1
    }
}

#[cw_serde]
pub enum VaultRebalancer {
    /// Only the contract admin can trigger rebalances.
    Admin {},
    /// Any delegated address decided by the admin can trigger rebalances.
    Delegate { rebalancer: Addr },
    /// Anyone can trigger rebalances, its the only option if the vault
    /// doesnt has an admin.
    Anyone {},
}

impl VaultRebalancer {
    pub fn new(rebalancer: VaultRebalancerInstantiateMsg, deps: Deps) -> Result<Self, ContractError> {
        use VaultRebalancerInstantiateMsg::*;
        match rebalancer {
            Delegate { rebalancer: x } => {
                let rebalancer = deps.api.addr_validate(&x)?;
                Ok(Self::Delegate { rebalancer })
            },
            Admin {} => Ok(Self::Admin {}),
            Anyone {} => Ok(Self::Anyone {}),
              
        }
    }
}

pub const VAULT_INFO: Item<VaultInfo> = Item::new("vault_info");
pub const VAULT_PARAMETERS: Item<VaultParameters> = Item::new("vault_parameters");


