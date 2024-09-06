use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Deps, Int128, QuerierWrapper, SignedDecimal256, Uint128};
use cw_storage_plus::Item;
use osmosis_std::types::osmosis::{
    concentratedliquidity::v1beta1::Pool,
    poolmanager::v1beta1::PoolmanagerQuerier
};

use readonly;

use crate::{constants::MIN_TICK, error::ContractError, msg::{VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg}};
use crate::constants::MAX_TICK;
use std::{cmp::min_by_key, str::FromStr};

#[cw_serde] #[readonly::make]
pub struct Weight(pub Decimal);
impl Weight {
    const MAX: Decimal = Decimal::one();

    pub fn new(value: &String) -> Option<Self> {
        let value = Decimal::from_str(value).ok()?;
        (value <= Self::MAX).then_some(Self(value))
    }
    
    pub fn mul_dec(&self, value: &Decimal) -> Decimal {
        // Invariant: A weight product wont ever overflow.
        value.checked_mul(self.0).unwrap()
    }

    pub fn is_zero(&self) -> bool { self.0 == Decimal::zero() }
    pub fn is_max(&self) -> bool { self.0 == Weight::MAX }

}

#[cw_serde] #[readonly::make]
pub struct PositiveDecimal(pub Decimal);
impl PositiveDecimal {
    pub fn new(value: &Decimal) -> Option<Self> {
        (value != Decimal::zero()).then_some(Self(*value))
    }

    pub fn floorlog10(&self) -> i32 {
        let x: u128 = self.0.atomics().into();
        // Invariant: `u128::ilog10(u128::MAX)` fits in `i32`.
        let x: i32 = x.ilog10().try_into().unwrap();
        // Invariant: `ilog10(1) - 18 = 0 - 18` fits in `i32`.
        let x = x.checked_sub(18).unwrap();
        // Invariant: `floor(log10(u128::MAX)) - 18 =  20` and
        //            `floor(log10(1))         - 18 = -18`
        assert!((-18..=20).contains(&x));
        x
    }
}

// TODO Check proof for output type `i32`, not `i64`.
pub fn price_function_inv(p: &Decimal) -> i32 {

    let maybe_neg_pow = |exp: i32| {
        let ten = SignedDecimal256::from_str("10").unwrap();
        if exp >= 0 {
            // Invariant: We just verified that `exp` is unsigned.
            let exp: u32 = exp.try_into().unwrap();
            ten.checked_pow(exp).ok()
        } else {
            SignedDecimal256::one().checked_div(
                ten.checked_pow(exp.unsigned_abs()).ok()?
            ).ok()
        }
    };

    let compute_price_inverse = |p| {
        let floor_log_p = PositiveDecimal::new(p)?.floorlog10();
        let x = floor_log_p.checked_mul(9)?.checked_sub(1)?;

        let x = maybe_neg_pow(floor_log_p)?
            .checked_mul(SignedDecimal256::from_str(&x.to_string()).ok()?).ok()?
            .checked_add((*p).try_into().ok()?).ok()?;

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

    pub fn current_tick(&self, querier: &QuerierWrapper) -> i32 {
        // TODO Prove and use safe conversions.
        self.to_pool(querier).current_tick as i32
    }

    pub fn tick_spacing(&self, querier: &QuerierWrapper) -> i32 {
        // TODO: Use safe conversions.
        // Invariant: Wont overflow under reasonable conditions.
        self.to_pool(querier).tick_spacing as i32
    }

    /// Min possible tick taking into account the pool tick spacing.
    pub fn min_valid_tick(&self, querier: &QuerierWrapper) -> i32 {
        let spacing = self.tick_spacing(querier);
        // Invarint: Wont overflow because `i64::MIN <<< MIN_TICK`.
        ((MIN_TICK + spacing + 1)/spacing) * spacing
    }

    /// Max possible tick taking into account the pool tick spacing.
    pub fn max_valid_tick(&self, querier: &QuerierWrapper) -> i32 {
        let spacing = self.tick_spacing(querier);
        (MAX_TICK/spacing) * spacing
    }

    // TODO Unsafe operations to prove here. TODO Prove function semantics.
    pub fn closest_valid_tick(&self, value: i32, querier: &QuerierWrapper) -> i32 {
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
        let p = PoolmanagerQuerier::new(querier)
            .spot_price(pool.id, pool.token0, pool.token1)
            .unwrap() // Invariant: We already verified the params are proper.
            .spot_price;

        // Invariant: We know that `querier.spot_price(...)`
        //            returns valid `Decimal` prices.
        Decimal::from_str(&p).unwrap()
    }

    pub fn denom0(&self, querier: &QuerierWrapper) -> String {
        self.to_pool(querier).token0
    }

    pub fn denom1(&self, querier: &QuerierWrapper) -> String {
        self.to_pool(querier).token1
    }

    pub fn denoms(&self, querier: &QuerierWrapper) -> (String, String) {
        (self.denom0(querier), self.denom1(querier))
    }
}

#[cw_serde] #[readonly::make]
pub struct PriceFactor(pub Decimal);
impl PriceFactor {
    pub fn new(value: &String) -> Option<Self> {
        let value = Decimal::from_str(value).ok()?;
        (value >= Decimal::one()).then_some(Self(value))
    }

    pub fn is_one(&self) -> bool { self.0 == Decimal::one() }

    // What even was this?
    pub fn mul_or_max(&self, price: &Decimal) -> Decimal {
        unimplemented!() 
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
}

impl VaultParameters {
    pub fn new(
        params: VaultParametersInstantiateMsg,
    ) -> Result<Self, ContractError> {

        let base_factor = PriceFactor::new(&params.base_factor)
            .ok_or(ContractError::InvalidPriceFactor(params.base_factor))?;
        
        let limit_factor = PriceFactor::new(&params.limit_factor)
            .ok_or(ContractError::InvalidPriceFactor(params.limit_factor))?;

        let full_range_weight = Weight::new(&params.full_range_weight)
            .ok_or(ContractError::InvalidWeight(params.full_range_weight))?;

        
        // NOTE: We dont support vaults with idle capital for now.
        match (full_range_weight.is_zero(), base_factor.is_one(), limit_factor.is_one()) {
            (true, true, true) => Err(ContractError::ContradictoryConfig {
                reason: "All vault parameters will produce null positions, all capital would be idle".into()
            }),
            (true, true, _) => Err(ContractError::ContradictoryConfig { 
                reason: "A vault without balanced orders will have idle capital".into()
            }),
            (_, _, true) => Err(ContractError::ContradictoryConfig { 
                reason: "A vault without a limit order will have idle capital".into()
            }),
            (_, true, _) if !full_range_weight.is_max() => Err(ContractError::ContradictoryConfig { 
                reason: "If the vault doenst have a base order, the full range weight should be 1".into()
            }),
            (_, false, _) if full_range_weight.is_max() => Err(ContractError::ContradictoryConfig { 
                reason: "If the full range weight is 1, the base factor should also be".into()
            }),
            _ => Ok(())
        }?;

        Ok(VaultParameters {
            base_factor, limit_factor, full_range_weight
        })
    }
}

#[cw_serde] #[readonly::make]
pub struct VaultInfo {
    #[readonly] pub pool_id: PoolId,
    pub admin: Option<Addr>,
    pub rebalancer: VaultRebalancer,
}

impl VaultInfo {
    pub fn new(info: VaultInfoInstantiateMsg, deps: Deps) -> Result<Self, ContractError> {
        let pool_id = PoolId::new(info.pool_id, &deps.querier)
            .ok_or(ContractError::InvalidPoolId(info.pool_id))?;

        assert!(pool_id.0 == info.pool_id);

        let rebalancer = VaultRebalancer::new(info.rebalancer, deps)?;

        let admin = if let Some(admin) = info.admin {
            Some(deps.api.addr_validate(&admin)
                .map_err(|_| ContractError::InvalidAdminAddress(admin))?)
        } else { 
            match rebalancer {
                VaultRebalancer::Anyone {} => Ok(None),
                _ => Err(ContractError::ContradictoryConfig { 
                    reason: "If admin is none, the rebalancer can only be anyone".into()
                })
            }?
        };

        Ok(VaultInfo { pool_id, rebalancer, admin })
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
                let rebalancer = deps.api.addr_validate(&x)
                    .map_err(|_| ContractError::InvalidDelegateAddress(x))?;
                Ok(Self::Delegate { rebalancer })
            },
            Admin {} => Ok(Self::Admin {}),
            Anyone {} => Ok(Self::Anyone {}),
              
        }
    }
}

#[cw_serde]
pub struct VaultState {
    // Position Ids are optional because: 
    // 1. Positions are only created on rebalances.
    // 2. If any of the vault positions is null, then those should 
    //    be `None`, see `VaultParameters`.
    pub full_range_position_id: Option<u64>,
    pub base_position_id: Option<u64>,
    pub limit_position_id: Option<u64>
}

impl Default for VaultState {
    fn default() -> Self {
        Self::new()
    }
}

impl VaultState {
    pub fn new() -> Self {
        Self {
            full_range_position_id: None,
            base_position_id: None,
            limit_position_id: None
        }
    }
}

/// VAULT_INFO Holds non-mathematical generally immutable information 
/// about the vault. Its generally immutable as in it can only be
/// changed by the vault admin, but its state cant be changed with
/// any business logic.
pub const VAULT_INFO: Item<VaultInfo> = Item::new("vault_info");

/// VAULT_PARAMETERS Holds mathematical generally immutable information 
/// about the vault. Its generally immutable as in it can only be
/// changed by the vault admin, but its state cant be changed with
/// any business logic.
pub const VAULT_PARAMETERS: Item<VaultParameters> = Item::new("vault_parameters");

/// VAULT_STATE Holds any vault state that can and will be changed
/// with contract business logic.
pub const VAULT_STATE: Item<VaultState> = Item::new("vault_state");
