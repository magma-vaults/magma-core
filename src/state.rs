use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Deps, QuerierWrapper, Timestamp, Uint128};
use cw_storage_plus::Item;
use osmosis_std::types::osmosis::{
    concentratedliquidity::v1beta1::Pool, poolmanager::v1beta1::PoolmanagerQuerier,
};
use readonly;
use crate::constants::{MAX_PROTOCOL_FEE, MAX_TICK};
use crate::error::InstantiationError;
use crate::{
    constants::MIN_TICK,
    msg::{VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg},
};
use std::{cmp::min_by_key, str::FromStr};

#[cw_serde]
#[readonly::make]
pub struct Weight(pub Decimal);
impl Weight {
    pub const MAX: Decimal = Decimal::one();

    pub fn new(value: &str) -> Option<Self> {
        let value = Decimal::from_str(value).ok()?;
        (value <= Self::MAX).then_some(Self(value))
    }

    pub fn mul_dec(&self, value: &Decimal) -> Decimal {
        // Invariant: A weight product wont ever overflow.
        value.checked_mul(self.0).unwrap()
    }

    pub fn mul_raw(&self, value: Uint128) -> Decimal {
        self.mul_dec(&Decimal::raw(value.into()))
    }

    pub fn max() -> Self {
        Self(Self::MAX)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == Decimal::zero()
    }

    pub fn is_max(&self) -> bool {
        self.0 == Weight::MAX
    }
}

impl TryFrom<Decimal> for Weight {
    type Error = ();
    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        if value > Self::MAX { Err(()) } 
        else { Ok(Self(value)) }
    }
}

#[cw_serde]
#[readonly::make]
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

#[cw_serde]
#[readonly::make]
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
        // Invariant: We already verified that the id refers to a valid pool the
        //            moment we constructed `self`.
        querier
            .pool(self.0).unwrap()
            .pool.unwrap()
            .try_into().unwrap()
    }

    pub fn price(&self, querier: &QuerierWrapper) -> Decimal {
        let pool = self.to_pool(querier);
        // Invariant: We already verified the params are proper the moment we constructed `self`.
        let p = PoolmanagerQuerier::new(querier)
            .spot_price(pool.id, pool.token0, pool.token1)
            .unwrap()
            .spot_price;

        // Invariant: We know that `querier.spot_price(...)` returns valid `Decimal` prices.
        Decimal::from_str(&p).unwrap()
    }
}

#[cw_serde]
#[readonly::make]
pub struct PriceFactor(pub Decimal);
impl PriceFactor {
    pub fn new(value: &str) -> Option<Self> {
        let value = Decimal::from_str(value).ok()?;
        (value >= Decimal::one()).then_some(Self(value))
    }

    pub fn is_one(&self) -> bool {
        self.0 == Decimal::one()
    }
}

#[cw_serde]
#[readonly::make]
pub struct ProtocolFee(pub Weight);
impl ProtocolFee {
    pub fn max() -> Decimal { *MAX_PROTOCOL_FEE }

    pub fn new(value: &str) -> Option<Self> {
        let value = Weight::new(value)?;
        (value.0 <= Self::max()).then_some(Self(value))
    }
}

impl Default for ProtocolFee {
    fn default() -> Self {
        // Invariant: Wont panic, `ProtocolFee::MAX` is 0.1, 
        Self::new("0.05").unwrap()
    } 
}

#[cw_serde]
pub struct VaultParameters {
    /// Price factor for the base order. Thus, if the current price is `p`,
    /// then the base position will have range `[p/base_factor, p*base_factor]`.
    /// if `base_factor == PriceFactor(Decimal::one())`, then the vault wont
    /// have a base order.
    pub base_factor: PriceFactor,
    /// Price factor for the limit order. Thus, if the current price is `p`,
    /// then the limit position will have either range `[p/limit_factor, p]` or
    /// `[p, p*limit_factor]`. If `limit_factor == PriceFactor(Decimal::one())`,
    /// then the vault wont have aa limit order, and will just hold remaining
    /// tokens.
    pub limit_factor: PriceFactor,
    /// Decimal weight, zero if we dont want a full range position.
    pub full_range_weight: Weight,
}

impl VaultParameters {
    pub fn new(params: VaultParametersInstantiateMsg) -> Result<Self, InstantiationError> {
        use InstantiationError::*;
        let base_factor = PriceFactor::new(&params.base_factor)
            .ok_or(InvalidPriceFactor(params.base_factor))?;

        let limit_factor = PriceFactor::new(&params.limit_factor)
            .ok_or(InvalidPriceFactor(params.limit_factor))?;

        let full_range_weight = Weight::new(&params.full_range_weight)
            .ok_or(InvalidWeight(params.full_range_weight))?;

        // NOTE: We dont support vaults with idle capital nor less than 3 positions for now.
        //       Integrating both options is trivial, but we keep it simple for the v1.
        match (
            full_range_weight.is_zero(),
            base_factor.is_one(),
            limit_factor.is_one(),
        ) {
            (false, false, false) => Ok(()),
            (true, true, true) => Err(ContradictoryConfig {
                reason:
                    "All vault parameters will produce null positions, all capital would be idle"
                        .into(),
            }),
            (true, true, _) => Err(ContradictoryConfig {
                reason: "A vault without balanced orders will have idle capital".into(),
            }),
            (_, _, true) => Err(ContradictoryConfig {
                reason: "A vault without a limit order will have idle capital".into(),
            }),
            (_, true, _) if !full_range_weight.is_max() => {
                Err(ContradictoryConfig {
                    reason:
                        "If the vault doenst have a base order, the full range weight should be 1"
                            .into(),
                })
            }
            (_, false, _) if full_range_weight.is_max() => {
                Err(ContradictoryConfig {
                    reason: "If the full range weight is 1, the base factor should also be".into(),
                })
            }
            _ => Err(ContradictoryConfig {
                reason: "We dont support vaults with less than 3 positions for now".into()
            }),
        }?;

        Ok(VaultParameters {
            base_factor,
            limit_factor,
            full_range_weight,
        })
    }
}

#[cw_serde]
#[readonly::make]
pub struct VaultInfo {
    #[readonly]
    pub pool_id: PoolId,
    pub admin: Option<Addr>,
    pub rebalancer: VaultRebalancer,
}

impl VaultInfo {
    pub fn new(info: VaultInfoInstantiateMsg, deps: Deps) -> Result<Self, InstantiationError> {
        use InstantiationError::*;
        let pool_id = PoolId::new(info.pool_id, &deps.querier)
            .ok_or(InvalidPoolId(info.pool_id))?;

        assert!(pool_id.0 == info.pool_id);

        let rebalancer = VaultRebalancer::new(info.rebalancer, deps)?;

        let admin = if let Some(admin) = info.admin {
            Some(
                deps.api
                    .addr_validate(&admin)
                    .map_err(|_| InvalidAdminAddress(admin))?,
            )
        } else {
            match rebalancer {
                VaultRebalancer::Anyone { .. } => Ok(None),
                _ => Err(ContradictoryConfig {
                    reason: "If admin is none, the rebalancer can only be anyone".into(),
                }),
            }?
        };

        Ok(VaultInfo {
            pool_id,
            rebalancer,
            admin
        })
    }

    pub fn demon0(&self, querier: &QuerierWrapper) -> String {
        self.pool_id.to_pool(querier).token0
    }

    pub fn demon1(&self, querier: &QuerierWrapper) -> String {
        self.pool_id.to_pool(querier).token1
    }

    pub fn denoms(&self, querier: &QuerierWrapper) -> (String, String) {
        (self.demon0(querier), self.demon1(querier))
    }

    pub fn pool(&self, querier: &QuerierWrapper) -> Pool {
        self.pool_id.to_pool(querier)
    }

    pub fn current_tick(&self, querier: &QuerierWrapper) -> i32 {
        // Invariant: Wont panic as max and min possible ticks below 2**31 - 1.
        self.pool(querier).current_tick.try_into().unwrap()
    }

    pub fn tick_spacing(&self, querier: &QuerierWrapper) -> i32 {
        // Invariant: Wont panic as max and min possible ticks below 2**31 - 1.
        self.pool(querier).tick_spacing.try_into().unwrap()
    }

    /// Min possible tick taking into account the pool tick spacing.
    pub fn min_valid_tick(&self, querier: &QuerierWrapper) -> i32 {
        let spacing = self.tick_spacing(querier);
        // Invarint: Wont overflow because `i64::MIN <<< MIN_TICK`.

        // Invariant: Wont panic.
        // Proof: Division wont fail, as `spacing` is always positive.
        //        Additions wont overflow, even for unreasonable tick
        //        spacings. Multiplication by spacing wont overflow,
        //        as we just divided by it.
        MIN_TICK
            .checked_add(spacing)
            .and_then(|x| x.checked_add(1))
            .and_then(|x| x.checked_div(spacing))
            .and_then(|x| x.checked_mul(spacing))
            .unwrap()
    }

    /// Max possible tick taking into account the pool tick spacing.
    pub fn max_valid_tick(&self, querier: &QuerierWrapper) -> i32 {
        let spacing = self.tick_spacing(querier);
        // Invariant: Wont panic, as `spacing` is always positive.
        MAX_TICK
            .checked_div(spacing)
            .and_then(|x| x.checked_mul(spacing))
            .unwrap()
    }

    // TODO: Document and lift to `i64`, as those computations could panic
    //       under unreasonable input values. I dont care for that for now,
    //       I'll just assume `value` is reasonable for now.
    pub fn closest_valid_tick(&self, value: i32, querier: &QuerierWrapper) -> i32 {
        let spacing = self.tick_spacing(querier);

        let lower = value
            .checked_div(spacing)
            .and_then(|x| x.checked_mul(spacing))
            .unwrap();

        let upper = value
            .checked_div(spacing)
            .and_then(|x| x.checked_add(1))
            .and_then(|x| x.checked_mul(spacing))
            .unwrap();

        let closest = min_by_key(lower, upper, |x| (x.checked_sub(value).unwrap()).abs());

        if closest < MIN_TICK {
            self.min_valid_tick(querier)
        } else if closest > MAX_TICK {
            self.max_valid_tick(querier)
        } else {
            closest
        }
    }
}

/// See [`VaultRebalancerInstantiateMsg`].
#[cw_serde]
pub enum VaultRebalancer {
    Admin {},
    Delegate { rebalancer: Addr },
    Anyone { 
        price_factor_before_rebalance: PriceFactor,
        time_before_rabalance: Timestamp
    },
}

impl VaultRebalancer {
    pub fn new(
        rebalancer: VaultRebalancerInstantiateMsg,
        deps: Deps,
    ) -> Result<Self, InstantiationError> {
        use VaultRebalancerInstantiateMsg::*;
        use InstantiationError::*;

        match rebalancer {
            Delegate { rebalancer: x } => {
                let rebalancer = deps
                    .api
                    .addr_validate(&x)
                    .map_err(|_| InvalidDelegateAddress(x))?;
                Ok(Self::Delegate { rebalancer })
            }
            Admin {} => Ok(Self::Admin {}),
            Anyone { seconds_before_rabalance, price_factor_before_rebalance } => {
                Ok(Self::Anyone {
                    price_factor_before_rebalance: PriceFactor::new(&price_factor_before_rebalance)
                        .ok_or(InvalidPriceFactor(price_factor_before_rebalance))?,
                    time_before_rabalance: Timestamp::from_seconds(seconds_before_rabalance)
                })
            }
        }
    }
}

#[cw_serde]
pub enum PositionType { FullRange, Base, Limit }

type MaybePositionId = Option<u64>;

// TODO: The bind can be stricter, as the second field can only change
//       in one direction.
#[cw_serde]
pub struct StateSnapshot {
    pub last_price: Decimal, 
    pub last_timestamp: Timestamp
}

#[cw_serde]
#[derive(Default)]
pub struct VaultState {
    /// Position Ids are optional because:
    /// 1. Positions are only created on rebalances.
    /// 2. If any of the vault positions is null, then those should
    ///    be `None`, see [`VaultParameters`].
    pub full_range_position_id: MaybePositionId,
    pub base_position_id: MaybePositionId,
    pub limit_position_id: MaybePositionId,

    /// last price and last timestamp since the last rebalance. Optional as it
    /// requires a first rebalance to happen to be set. After that, both will
    /// always be set.
    pub last_price_and_timestamp: Option<StateSnapshot>
}

impl VaultState {
    pub fn from_position_type(&self, position_type: PositionType) -> MaybePositionId {
        match position_type {
            PositionType::FullRange => self.full_range_position_id,
            PositionType::Base => self.base_position_id,
            PositionType::Limit => self.limit_position_id,
        }
    }
}


#[cw_serde]
#[derive(Default)]
pub struct FeesInfo {
    pub protocol_fee: ProtocolFee,
    pub protocol_tokens0_owned: Uint128,
    pub protocol_tokens1_owned: Uint128,
    pub admin_fee: ProtocolFee,
    pub admin_tokens0_owned: Uint128,
    pub admin_tokens1_owned: Uint128,
}

impl FeesInfo {
    pub fn new(admin_fee: String, vault_info: &VaultInfo) -> Result<FeesInfo, InstantiationError> {
        let admin_fee = ProtocolFee::new(&admin_fee)
            .ok_or(InstantiationError::InvalidAdminFee {
                max: ProtocolFee::max().to_string(),
                got: admin_fee 
            })?;

        if !admin_fee.0.is_zero() && vault_info.admin.is_none() {
            Err(InstantiationError::AdminFeeWithoutAdmin {})
        } else {
            Ok(FeesInfo { admin_fee, ..FeesInfo::default() })
        }
    }
}

#[cw_serde]
#[derive(Default)]
pub struct FundsInfo {
    pub available_balance0: Uint128,
    pub available_balance1: Uint128
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

pub const FEES_INFO: Item<FeesInfo> = Item::new("fees_info");

/// FUNDS_INFO Refers to the known funds available to the contract,
/// without counting protocol/admin fees.
pub const FUNDS_INFO: Item<FundsInfo> = Item::new("funds_info");

