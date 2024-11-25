use crate::constants::{
    DEFAULT_PROTOCOL_FEE, DEFAULT_VAULT_CREATION_COST, MAX_PROTOCOL_FEE, MAX_TICK,
    MAX_VAULT_CREATION_COST, TWAP_SECONDS, VAULT_CREATION_COST_DENOM,
};
use crate::do_some;
use crate::error::{InstantiationError, ProtocolOperationError};
use crate::{
    constants::MIN_TICK,
    msg::{VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg},
};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Deps, Env, MessageInfo, QuerierWrapper, Timestamp, Uint128};
use cw_storage_plus::Item;
use osmosis_std::types::osmosis::twap::v1beta1::TwapQuerier;
use osmosis_std::types::osmosis::{
    concentratedliquidity::v1beta1::Pool, poolmanager::v1beta1::PoolmanagerQuerier,
};
use readonly;
use std::i32;
use std::{cmp::min_by_key, str::FromStr};

#[cw_serde]
#[readonly::make]
pub struct Weight(pub Decimal);
impl Weight {
    pub const MAX: Decimal = Decimal::one();

    pub fn new(value: &Uint128) -> Option<Self> {
        let value = Decimal::raw(value.u128());
        (value <= Self::MAX).then_some(Self(value))
    }

    pub fn permille(value: u64) -> Option<Self> {
        let value = Decimal::permille(value);
        (value <= Self::MAX).then_some(Self(value))
    }

    pub fn mul_dec(&self, value: &Decimal) -> Decimal {
        // Invariant: A weight product wont ever overflow.
        value.checked_mul(self.0).unwrap()
    }

    pub fn mul_raw(&self, value: Uint128) -> Decimal {
        self.mul_dec(&Decimal::raw(value.into()))
    }

    pub fn zero() -> Self {
        Self(Decimal::zero())
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
        Self::new(&value.atomics()).ok_or(())
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
        // Invariant: We already verified that the id refers to a valid pool the
        //            moment we constructed `self`.
        PoolmanagerQuerier::new(querier)
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

    pub fn twap(&self, querier: &QuerierWrapper, env: &Env) -> Option<Decimal> {
        let start_time = env.block.time;
        // Invariant: Wont overflow as `env.block.time` is reasonable.
        let osmosis_start_time = Some(osmosis_std::shim::Timestamp {
            seconds: start_time.seconds().saturating_sub(TWAP_SECONDS).try_into().unwrap(),
            nanos: 0
        });
        let pool = self.to_pool(querier);

        // Invariant: Will only return `None` if `pool` was recently created, as
        //            we already ensured that `self` is valid during instantiation
        //            and that the start time is in the near past.
        let p = TwapQuerier::new(querier)
            .geometric_twap_to_now(self.0, pool.token0, pool.token1, osmosis_start_time)
            .ok()?
            .geometric_twap;

        // Invariant: We know `.geometric_twap_to_now(...)` returns valid `Decimal` values.
        Some(Decimal::from_str(&p).unwrap())
    }
}

#[cw_serde]
#[readonly::make]
pub struct PriceFactor(pub Decimal);
impl PriceFactor {
    pub fn new(value: &Uint128) -> Option<Self> {
        let value = Decimal::raw(value.u128());
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
    pub fn max() -> Decimal {
        MAX_PROTOCOL_FEE
    }

    pub fn new(value: &Uint128) -> Option<Self> {
        let value = Weight::new(value)?;
        (value.0 <= Self::max()).then_some(Self(value))
    }

    pub fn zero() -> ProtocolFee {
        Self(Weight::zero())
    }
}

impl Default for ProtocolFee {
    fn default() -> Self {
        // Invariant: Wont panic as the const is in [0, 1].
        Self(Weight::try_from(DEFAULT_PROTOCOL_FEE).unwrap())
    }
}

#[cw_serde]
#[readonly::make]
pub struct VaultCreationCost(pub Uint128);
impl VaultCreationCost {
    pub fn max() -> Uint128 {
        MAX_VAULT_CREATION_COST
    }

    pub fn new(value: Uint128) -> Option<Self> {
        (value <= Self::max()).then_some(Self(value))
    }
}

impl Default for VaultCreationCost {
    fn default() -> Self {
        // Invariant: Wont panic, as the const is clearly below `Self::max()`.
        Self::new(DEFAULT_VAULT_CREATION_COST).unwrap()
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
    /// then the vault wont have a limit order, and will just hold remaining
    /// tokens.
    pub limit_factor: PriceFactor,
    /// Exact liquidity weight to put into the full range order. 
    /// Zero if we dont want a full range position.
    pub full_range_weight: Weight
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
                reason: "All vault parameters will produce null positions, all capital would be idle".into()
            }),
            (true, true, _) => Err(ContradictoryConfig {
                reason: "A vault without balanced orders will have idle capital".into()
            }),
            (_, _, true) => Err(ContradictoryConfig {
                reason: "A vault without a limit order will have idle capital".into()
            }),
            (_, true, _) if !full_range_weight.is_max() => Err(ContradictoryConfig {
                reason: "If the vault doenst have a base order, the full range weight should be 1".into()
            }),
            (_, false, _) if full_range_weight.is_max() => Err(ContradictoryConfig {
                reason: "If the full range weight is 1, the base factor should also be".into()
            }),
            _ => Err(ContradictoryConfig {
                reason: "We dont support vaults with less than 3 positions for now".into()
            })
        }?;

        Ok(VaultParameters { base_factor, limit_factor, full_range_weight })
    }
}

#[cw_serde]
#[readonly::make]
pub struct VaultInfo {
    #[readonly]
    pub pool_id: PoolId,
    pub admin: Option<Addr>,
    pub proposed_new_admin: Option<Addr>,
    pub rebalancer: VaultRebalancer
}

impl VaultInfo {
    pub fn new(info: VaultInfoInstantiateMsg, deps: Deps) -> Result<Self, InstantiationError> {
        use InstantiationError::*;
        let pool_id = PoolId::new(info.pool_id, &deps.querier).ok_or(InvalidPoolId(info.pool_id))?;

        let rebalancer = VaultRebalancer::new(info.rebalancer, deps)?;

        let admin = if let Some(admin) = info.admin {
            Some(deps.api
                .addr_validate(&admin)
                .map_err(|_| InvalidAdminAddress(admin))?,
            )
        } else { None };

        rebalancer.rebalancer_consistent_with_admin(&admin)?;

        Ok(VaultInfo {
            pool_id,
            rebalancer,
            admin,
            proposed_new_admin: None
        })
    }
    
    pub fn propose_new_admin(self, new_admin: String, deps: Deps) -> Option<Self> {
        let proposed_new_admin = Some(deps.api.addr_validate(&new_admin).ok()?);
        Some(Self { proposed_new_admin, ..self })
    }

    pub fn unset_proposed_new_admin(self) -> Self {
        Self { proposed_new_admin: None, ..self }
    }

    pub fn confirm_new_admin(self) -> Self {
        let admin = self.proposed_new_admin;
        Self { admin, proposed_new_admin: None, ..self }
    }

    pub fn burn_admin(self) -> Self {
        Self { admin: None, ..self }
    }

    pub fn change_rebalancer(
        self,
        new_rebalancer: VaultRebalancerInstantiateMsg,
        deps: Deps
    ) -> Result<Self, InstantiationError> {
        let rebalancer = VaultRebalancer::new(new_rebalancer, deps)?;
        rebalancer.rebalancer_consistent_with_admin(&self.admin)?;
        Ok(Self { rebalancer, ..self })
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

        // Invariant: Wont panic.
        // Proof: Division wont fail, as `spacing` is always $\geq$ 1.
        //        Additions wont overflow, even for unreasonable tick
        //        spacings. Multiplication by spacing wont overflow,
        //        as we just divided by it.
        do_some!(MIN_TICK
            .checked_add(spacing)?
            .checked_add(1)?
            .checked_div(spacing)?
            .checked_mul(spacing)?
        ).unwrap()
    }

    /// Max possible tick taking into account the pool tick spacing.
    pub fn max_valid_tick(&self, querier: &QuerierWrapper) -> i32 {
        let spacing = self.tick_spacing(querier);

        // Invariant: Wont panic, as `spacing` is always $\geq$ 1.
        MAX_TICK
            .checked_div(spacing)
            .and_then(|x| x.checked_mul(spacing))
            .unwrap()
    }

    pub fn closest_valid_tick(&self, value: i32, querier: &QuerierWrapper) -> i32 {
        let spacing = self.tick_spacing(querier);

        // Invariant: Wont overflow, as `floor(value/spacing) * spacing $\leq$ value`.
        let lower = value
            .checked_div(spacing)
            .and_then(|x| x.checked_mul(spacing))
            .unwrap();

        // Invariant: Could only overflow if the upper closest valid
        //            tick did not fit in `i32`, in which case we still
        //            would be using `self.max_valid_tick` (see below).
        let upper = value
            .checked_div(spacing)
            .and_then(|x| x.checked_add(1))
            .and_then(|x| x.checked_mul(spacing))
            .unwrap_or(i32::MAX);

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
    Delegate {
        rebalancer: Addr,
    },
    Anyone {
        price_factor_before_rebalance: PriceFactor,
        time_before_rabalance: Timestamp,
    }
}

impl VaultRebalancer {
    pub fn new(
        rebalancer: VaultRebalancerInstantiateMsg,
        deps: Deps
    ) -> Result<Self, InstantiationError> {
        use InstantiationError::*;
        use VaultRebalancerInstantiateMsg::*;

        match rebalancer {
            Delegate { rebalancer } => {
                let rebalancer = deps
                    .api
                    .addr_validate(&rebalancer)
                    .map_err(|_| InvalidDelegateAddress(rebalancer))?;
                Ok(Self::Delegate { rebalancer })
            }
            Admin {} => Ok(Self::Admin {}),
            Anyone {
                seconds_before_rebalance, price_factor_before_rebalance
            } => Ok(Self::Anyone {
                price_factor_before_rebalance: PriceFactor::new(&price_factor_before_rebalance)
                    .ok_or(InvalidPriceFactor(price_factor_before_rebalance))?,
                time_before_rabalance: Timestamp::from_seconds(seconds_before_rebalance.into())
            })
        }
    }

    fn rebalancer_consistent_with_admin(
        &self,
        current_vault_admin: &Option<Addr>
    ) -> Result<(), InstantiationError> {
        if current_vault_admin.is_none() {
            match self {
                VaultRebalancer::Anyone { .. } => Ok(()),
                _ => Err(InstantiationError::ContradictoryConfig {
                    reason: "If admin is none, the rebalancer can only be anyone".into(),
                })
            }
        } else { Ok(()) }
    }
}

#[cw_serde]
pub enum PositionType { FullRange, Base, Limit }

type MaybePositionId = Option<u64>;

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
            PositionType::Limit => self.limit_position_id
        }
    }
}

#[cw_serde]
#[derive(Default)]
pub struct FeesInfo {
    pub protocol_fee: ProtocolFee,
    pub protocol_tokens0_owned: Uint128,
    pub protocol_tokens1_owned: Uint128,
    pub protocol_vault_creation_cost: VaultCreationCost,
    pub protocol_vault_creation_tokens_owned: Uint128,
    pub admin_fee: ProtocolFee,
    pub admin_tokens0_owned: Uint128,
    pub admin_tokens1_owned: Uint128
}

impl FeesInfo {
    
    fn validate_vault_creation_cost(info: &MessageInfo) -> Result<Uint128, InstantiationError> {
        let vault_creation_cost = VaultCreationCost::default();

        let paid_amount = cw_utils::must_pay(info, VAULT_CREATION_COST_DENOM).unwrap_or_default();

        if paid_amount != vault_creation_cost.0 {
            Err(InstantiationError::VaultCreationCostNotPaid {
                cost: vault_creation_cost.0.into(),
                denom: VAULT_CREATION_COST_DENOM.into(),
                got: paid_amount.into()
            })
        } else { Ok(paid_amount) }
    }

    fn validate_admin_fee(admin_fee: Uint128, vault_info: &VaultInfo) -> Result<ProtocolFee, InstantiationError> {
        let admin_fee = ProtocolFee::new(&admin_fee).ok_or(InstantiationError::InvalidAdminFee {
            max: ProtocolFee::max().atomics(),
            got: admin_fee,
        })?;

        if !admin_fee.0.is_zero() && vault_info.admin.is_none() {
            Err(InstantiationError::AdminFeeWithoutAdmin {})
        } else { Ok(admin_fee) }
    }

    pub fn new(
        admin_fee: Uint128,
        vault_info: &VaultInfo,
        info: &MessageInfo
    ) -> Result<FeesInfo, InstantiationError> {
        let paid_amount = Self::validate_vault_creation_cost(info)?;
        let admin_fee = Self::validate_admin_fee(admin_fee, vault_info)?;

        Ok(FeesInfo {
            admin_fee,
            protocol_vault_creation_tokens_owned: paid_amount,
            ..FeesInfo::default()
        })
    }

    pub fn update_admin_fee(&self, admin_fee: Uint128, deps: Deps) -> Result<FeesInfo, InstantiationError> {
        // Invariant: Any state is present after instantitation.
        let vault_info = VAULT_INFO.load(deps.storage).unwrap();
        let admin_fee = Self::validate_admin_fee(admin_fee, &vault_info)?;
        Ok(FeesInfo { admin_fee, ..self.clone() })
    }

    pub fn update_protocol_fee(&self, protocol_fee: Uint128) -> Result<FeesInfo, ProtocolOperationError> {
        let protocol_fee = 
            ProtocolFee::new(&protocol_fee).ok_or(ProtocolOperationError::InvalidProtocolFee { 
                max: MAX_PROTOCOL_FEE.atomics(),
                got: protocol_fee
            })?;

        Ok(FeesInfo { protocol_fee, ..self.clone() })
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

/// FEES_INFO Holds any uncollected admin/protocol fees and fee parameters.
pub const FEES_INFO: Item<FeesInfo> = Item::new("fees_info");

/// FUNDS_INFO Refers to the known funds available to the contract,
/// without counting protocol/admin fees.
pub const FUNDS_INFO: Item<FundsInfo> = Item::new("funds_info");

