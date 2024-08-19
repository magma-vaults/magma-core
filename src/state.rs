use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Deps, Int128, OverflowError, QuerierWrapper, SignedDecimal, SignedDecimal256, Uint128};
use cw_storage_plus::Item;
use osmosis_std::types::osmosis::{
    concentratedliquidity::v1beta1::{FullTick, MsgCreatePosition, Pool, TickInfo},
    poolmanager::v1beta1::PoolmanagerQuerier
};

use readonly;

use crate::{constants::MIN_TICK, error::ContractError, msg::{VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg}};
use crate::constants::MAX_TICK;
use std::{any::type_name_of_val, cmp::min_by_key, str::FromStr};

#[cw_serde] #[readonly::make]
pub struct Weight(pub Decimal);
impl Weight {
    const MAX: Decimal = Decimal::one();

    pub fn new(value: String) -> Option<Self> {
        let value = Decimal::from_str(&value).ok()?;
        (value <= Self::MAX).then_some(Self(value))
    }
    
    pub fn mul_dec(&self, value: &Decimal) -> Decimal {
        // Invariant: A weight product wont ever overflow.
        value.checked_mul(self.0).unwrap()
    }

    pub fn is_zero(&self) -> bool { self.0 == Decimal::zero() }

}

#[cw_serde] #[readonly::make]
pub struct PositiveDecimal(pub Decimal);
impl PositiveDecimal {
    pub fn new(value: &Decimal) -> Option<Self> {
        (value != Decimal::zero()).then_some(Self(value.clone()))
    }

    pub fn floorlog10(&self) -> i32 {
        let x: u128 = self.0.atomics().into();
        // Invariant: `u128::ilog10(u128::MAX)` fits in `i32`.
        let x: i32 = x.ilog10().try_into().unwrap();
        // Invariant: `ilog10(1) - 18 = 0 - 18` fits in `i32`.
        let x = x.checked_sub(18).unwrap();
        // Invariant: `floor(log10(u128::MAX)) - 18 =  20` and
        //            `floor(log10(1))         - 18 = -18`
        assert!(-18 <= x && x <= 20);
        x
    }
}

// TODO Check proof for output type `i32`, not `i64`.
pub fn price_function_inv(p: &Decimal) -> i32 {

    let maybe_neg_pow = |exp: i32| {
        let ten = SignedDecimal256::new(10.into());
        if exp >= 0 {
            // Invariant: We just verified that `exp` is unsigned.
            let exp: u32 = exp.try_into().unwrap();
            ten.checked_pow(exp).ok()
        } else {
            SignedDecimal256::one().checked_div(
                ten.checked_pow(exp.unsigned_abs().into()).ok()?
            ).ok()
        }
    };

    let compute_price_inverse = |p| {
        let floor_log_p = PositiveDecimal::new(p)?.floorlog10();
        let x = floor_log_p.checked_mul(9)?.checked_sub(1)?;

        let x = maybe_neg_pow(floor_log_p)?
            .checked_mul(SignedDecimal256::new(x.into())).ok()?
            .checked_add(p.clone().try_into().ok()?).ok()?;

        let x = maybe_neg_pow(6i32.checked_sub(floor_log_p)?)?
            .checked_mul(x).ok()?;

        let x: Int128 = x.to_int_floor().try_into().ok()?;
        x.i128().try_into().ok()
    };

    // Invariant: Price function inverse computation doesnt overflow under i256.
    //     Proof: See whitepaper theorem 5.
    compute_price_inverse(p).unwrap()
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

    /// Min possible tick taking into account the pool tick spacing.
    pub fn min_valid_tick(&self, querier: &QuerierWrapper) -> i64 {
        let spacing = self.tick_spacing(querier);
        // Invarint: Wont overflow because `i64::MIN <<< MIN_TICK`.
        ((MIN_TICK + spacing + 1)/spacing) * spacing
    }

    /// Max possible tick taking into account the pool tick spacing.
    pub fn max_valid_tick(&self, querier: &QuerierWrapper) -> i64 {
        let spacing = self.tick_spacing(querier);
        (MAX_TICK/spacing) * spacing
    }

    pub fn closest_valid_tick(&self, value: i32, querier: &QuerierWrapper) -> i64 {
        let value: i64 = value.into();
        let spacing = self.tick_spacing(querier);
        let lower = (value/spacing) * spacing;
        // Invariant: Wont overflow because `i32::MAX <<< i64::MAX`
        let upper = (value/spacing + 1) * spacing;
        let closest = min_by_key(lower, upper, |x| (x - value).abs());

        if closest < MIN_TICK { self.min_valid_tick(querier) }
        else if closest > MAX_TICK { self.max_valid_tick(querier) }
        else { closest }
    }

    pub fn price(&self, querier: &QuerierWrapper) -> Decimal {
        let pool = self.to_pool(querier);
        let querier = PoolmanagerQuerier::new(querier);
        let p = querier
            .spot_price(pool.id, pool.token0, pool.token1)
            .unwrap() // Invariant: We already verified the params are proper.
            .spot_price;

        // Invariant: We know that `querier.spot_price(...)`
        //            returns valid `Decimal` prices.
        Decimal::from_str(&p).unwrap()
    }
}

#[cw_serde] #[readonly::make]
pub struct PriceFactor(pub Decimal);
impl PriceFactor {
    pub fn new(value: String) -> Option<Self> {
        let value = Decimal::from_str(&value).ok()?;
        (value >= Decimal::one()).then_some(Self(value))
    }

    pub fn is_one(&self) -> bool { self.0 == Decimal::one() }

    pub fn mul_or_max(&self, price: &Decimal) -> Decimal {
         
    }
}

pub fn raw<T: From<Uint128>>(d: &Decimal) -> T { d.atomics().into()  }




#[cw_serde]
pub struct VaultParameters {
    // Price factor for the base order. Thus, if the current price is `p`,
    // then the base position will have range `[p/base_factor, p*base_factor]`.
    // if `base_factor == PriceFactor(Decimal::one())`, then the vault wont
    // have a base order.
    pub base_factor: PriceFactor,
    // Price factor for the limit order. Thus, if the current price is `p`,
    // then the limit position will have either range `[p/limit_factor, p]` or
    // `[p, p*limit_factor]`. If `limit_factor == PriceFactor(Decimal::one())`,
    // then the vault wont have aa limit order, and will just hold remaining
    // tokens.
    pub limit_factor: PriceFactor,
    // Decimal weight, zero if we dont want a full range position.
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


