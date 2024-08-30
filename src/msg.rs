use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Coin, Uint128};

#[cw_serde]
pub struct VaultParametersInstantiateMsg {
    pub base_factor: String, // Decimal value, greater or equal to 1.
    pub limit_factor: String, // Decimal value, greater or equal to 1.
    pub full_range_weight: String // Decimal value, in range [0, 1].
}

#[cw_serde]
pub struct VaultInfoInstantiateMsg {
    pub pool_id: u64,
    pub vault_name: String,
    pub vault_symbol: String,
    pub admin: Option<String>,
    pub rebalancer: VaultRebalancerInstantiateMsg
}

#[cw_serde]
pub enum VaultRebalancerInstantiateMsg {
    /// Only the contract admin can trigger rebalances.
    Admin {},
    /// Any delegated address decided by the admin can trigger rebalances.
    Delegate { rebalancer: String },
    /// Anyone can trigger rebalances, its the only option if there isnt a
    /// vault manager.
    Anyone {}
}

#[cw_serde]
pub struct InstantiateMsg {
    pub vault_info: VaultInfoInstantiateMsg,
    pub vault_parameters: VaultParametersInstantiateMsg
}

#[cw_serde]
pub struct DepositMsg {
    pub amount0: String, // Decimal compatible value.
    pub amount1: String, // Decimal compatible value.
    pub amount0_min: String, // Decimal compatible value.
    pub amount1_min: String, // Decimal compatible value.
    pub to: String
}

#[cw_serde]
pub enum ExecuteMsg {
    Deposit(DepositMsg),
    Rebalance {}
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(VaultBalancesResponse)]
    VaultBalances {},
    #[returns(PositionBalancesWithFeesResponse)]
    PositionBalancesWithFees { position_type: PositionType }
}

#[cw_serde]
pub enum PositionType { FullRange, Base, Limit }

#[cw_serde]
pub struct CoinsPair(pub Coin, pub Coin);

impl CoinsPair {
    pub fn new(
        denom0: String, amount0: Uint128,
        denom1: String, amount1: Uint128
    ) -> Self {
        Self(
            Coin { denom: denom0, amount: amount0 },
            Coin { denom: denom1, amount: amount1 }
        )
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {

        // Invariant: Balances addition will never overflow because
        //            for that Coins supply would have to be larger
        //            than `Uint128::MAX`, but thats not possible.
        (self.0.denom == other.0.denom && self.1.denom == other.1.denom).then_some(
            CoinsPair(
                Coin {
                    denom: self.0.denom,
                    amount: self.0.amount.checked_add(other.0.amount).unwrap(),
                },
                Coin {
                    denom: self.1.denom,
                    amount: self.1.amount.checked_add(other.1.amount).unwrap(),
                }
            )
        )
    }
}

#[cw_serde]
pub struct VaultBalancesResponse {
    pub res: CoinsPair 
}

#[cw_serde]
pub struct PositionBalancesWithFeesResponse {
    pub res: CoinsPair
}


