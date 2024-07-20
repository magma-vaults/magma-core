use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, QuerierWrapper};
use cw_storage_plus::Item;
use osmosis_std::types::osmosis::{
    concentratedliquidity::v1beta1::Pool,
    poolmanager::v1beta1::PoolmanagerQuerier
};

use readonly;

/// 6 decimal point precision weight, represented internally as an `u32`.
#[cw_serde] #[readonly::make]
pub struct Weight(pub u32);
impl Weight {
    pub fn new(value: u32) -> Option<Self> {
        (value <= u32::pow(10, 6)).then_some(Self(value))
    }
}

#[cw_serde] #[readonly::make]
pub struct PositiveTick(pub u64);
impl PositiveTick {
    pub fn new(value: u64, pool_id: PoolId, querier: &QuerierWrapper) -> Option<Self> {
        let spacing = pool_id.to_pool(querier).tick_spacing;
        (value > 0 && value % spacing == 0).then_some(Self(value))
    }
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

    pub fn to_pool(self, querier: &QuerierWrapper) -> Pool {
        let querier = PoolmanagerQuerier::new(querier);
        // Invariant: We already verified that `id` refers to a valid pool.
        querier.pool(self.0).unwrap().pool.unwrap().try_into().unwrap()
    }
}


#[cw_serde]
pub struct VaultParameters {
    pub base_threshold: PositiveTick,
    pub base_position_id: u64,
    pub limit_threshold: PositiveTick,
    pub limit_position_id: u64,
    pub full_range_weight: Weight,
    pub full_range_position_id: u64
}

#[cw_serde] #[readonly::make]
pub struct VaultInfo {
    #[readonly] pub pool_id: PoolId,
    #[readonly] pub vault_name: String,
    #[readonly] pub vault_symbol: String,
    pub admin: Option<Addr>,
    pub rebalancer: VaultRebalancer,
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

pub const VAULT_INFO: Item<VaultInfo> = Item::new("vault_info");
pub const VAULT_PARAMETERS: Item<VaultParameters> = Item::new("vault_parameters");

