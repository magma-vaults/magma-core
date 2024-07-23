use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, QuerierWrapper, Deps};
use cw_storage_plus::Item;
use osmosis_std::types::osmosis::{
    concentratedliquidity::v1beta1::Pool,
    poolmanager::v1beta1::PoolmanagerQuerier
};

use readonly;

use crate::{constants::MIN_TICK, error::ContractError, msg::{VaultInfoInstantaiteMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg}};
use crate::constants::MAX_TICK;
use std::cmp::{self, min_by, min_by_key};

/// 6 decimal point precision weight, represented internally as an `u32`.
#[cw_serde] #[readonly::make]
pub struct Weight(pub u64);
impl Weight {
    pub fn new(value: u64) -> Option<Self> {
        (value <= u64::pow(10, 6)).then_some(Self(value))
    }

    const MAX: u64 = u64::pow(10, 6);
}

#[cw_serde] #[readonly::make]
pub struct NonNegTick(pub u64);

#[cw_serde] #[readonly::make]
pub struct Tick(pub i64);

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

    pub fn tick_spacing(&self, querier: &QuerierWrapper) -> u64 {
        self.to_pool(querier).tick_spacing
    }

    pub fn new_non_neg_tick(&self, value: u64, querier: &QuerierWrapper) -> Option<NonNegTick> {
        let spacing = self.tick_spacing(querier);
        (value % spacing == 0 && value <= MAX_TICK as u64).then_some(NonNegTick(value))
    }

    pub fn new_tick(&self, value: i64, querier: &QuerierWrapper) -> Option<Tick> {
        // Invariant: `spacing` will never be above `2**63`.
        let spacing = self.tick_spacing(querier) as i64;
        (value % spacing == 0 && value <= MAX_TICK && value >= MIN_TICK)
            .then_some(Tick(value))
    }

    pub fn closest_valid_tick(&self, value: i64, querier: &QuerierWrapper) -> Tick {
        let spacing = self.tick_spacing(querier) as i64;

        // Ceil on MIN, floor on MAX. 
        // Wont overflow as long as MIN and MAX are reasonable.
        let value_or_bound = |value|
            if value < MIN_TICK { ((MIN_TICK + spacing - 1)/spacing) * spacing }
            else if value > MAX_TICK { (MAX_TICK/spacing) * spacing }
            else { value };
        
        let value = value_or_bound(value);
        let floor = value_or_bound((value/spacing) * spacing);
        let ceil = value_or_bound((value/spacing + 1) * spacing);
        
        Tick(min_by_key(floor, ceil, |x| (x - value).abs()))
    }
}

impl Tick {
    pub fn abs(self) -> NonNegTick {
        // Invariant: Woknt overflow because `-2**63 > -2**64`.
        NonNegTick(self.0.abs() as u64)
    }
}

// pub struct Tick2(i64);
// impl Tick2 {
//     pub fn new(value: i64, pool_id: PoolId, querier: &QuerierWrapper) -> Option<Self> {
//         // Invariant: `spacing` will never be above `2**63`.
//         let spacing = pool_id.to_pool(querier).tick_spacing as i64;
//         (value % spacing == 0 && value <= MAX_TICK && value >= MIN_TICK)
//             .then_some(Self(value))
//     }
// 
//     pub fn floor_to_spacing(value: i64, pool_id: PoolId
// }



#[cw_serde]
pub struct VaultParameters {
    pub base_threshold: Tick,
    pub limit_threshold: Tick,
    pub full_range_weight: Weight,
    // Position Ids are optional because: 
    // 1. Positions are oly created on rebalances.
    // 2. If any of the params is 0, then the position for them might be None.
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
        let base_threshold = Tick::new(
            params.base_threshold, vault_info.pool_id.clone(), querier
        ).ok_or(ContractError::InvalidConfig {})?;

        let limit_threshold = Tick::new(
            params.limit_threshold, vault_info.pool_id.clone(), querier
        ).ok_or(ContractError::InvalidConfig {})?;

        let full_range_weight = Weight::new(params.full_range_weight)
            .ok_or(ContractError::InvalidConfig {})?;

        if base_threshold.0 == 0 &&
           limit_threshold.0 == 0 &&
           full_range_weight.0 != Weight::MAX 
        {
            return Err(ContractError::InvalidConfig {})
        }

        Ok(VaultParameters {
            base_threshold, limit_threshold, full_range_weight,
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
    pub fn new(info: VaultInfoInstantaiteMsg, deps: Deps) -> Result<Self, ContractError> {
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
            Delegate { rebalancer: x } => Ok(Self::Delegate {
                rebalancer: deps.api.addr_validate(&x)?
            }),
            Admin {} => Ok(Self::Admin {}),
            Anyone {} => Ok(Self::Anyone {}),
              
        }
    }
}

pub const VAULT_INFO: Item<VaultInfo> = Item::new("vault_info");
pub const VAULT_PARAMETERS: Item<VaultParameters> = Item::new("vault_parameters");

