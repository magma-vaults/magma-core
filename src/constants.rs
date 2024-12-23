use cosmwasm_std::{Decimal, Uint128};

pub const MIN_TICK: i32 = -108_000_000;
pub const MAX_TICK: i32 = 342_000_000;
pub const MIN_LIQUIDITY: Uint128 = Uint128::new(1000);
pub const TWAP_SECONDS: u64 = 60;
pub const POSITION_CREATION_SLIPPAGE: Decimal = Decimal::permille(999);

pub static PROTOCOL_ADDR: &str = "osmo1a8gd76fw6umx652v7cs73vnge2zju8s8hcm86t";
pub const DEFAULT_PROTOCOL_FEE: Decimal = Decimal::permille(50);
pub const MAX_PROTOCOL_FEE: Decimal = Decimal::permille(100);
/// USDC denom for mainnet.
pub const VAULT_CREATION_COST_DENOM: &str = "ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4";
/// 20 USDC max vault creation cost. Its only proper as USDC has 6 decimals.
pub const MAX_VAULT_CREATION_COST: Uint128 = Uint128::new(20_000_000);
pub const DEFAULT_VAULT_CREATION_COST: Uint128 = Uint128::new(5_000_000);
