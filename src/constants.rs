use std::str::FromStr;

use cosmwasm_std::{Addr, Decimal, Uint128};
use once_cell::sync::Lazy;

pub const MIN_TICK: i32 = -108_000_000;
pub const MAX_TICK: i32 = 342_000_000;
pub const MIN_LIQUIDITY: Uint128 = Uint128::new(1000);
pub static PROTOCOL: Lazy<Addr> = Lazy::new(|| Addr::unchecked("TODO"));
pub static MAX_PROTOCOL_FEE: Lazy<Decimal> = Lazy::new(|| Decimal::from_str("0.1").unwrap());

// NOTE: USDC denom for mainnet.
// pub const VAULT_CREATION_COST_DENOM: &str = "ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4";
// NOTE: USDC denom for testnet.
pub const VAULT_CREATION_COST_DENOM: &str = "ibc/DE6792CF9E521F6AD6E9A4BDF6225C9571A3B74ACC0A529F92BC5122A39D2E58";
// NOTE: 20 USDC max vault creation cost. Its only proper as USDC has 6 decimals.
pub static MAX_VAULT_CREATION_COST: Uint128 = Uint128::new(20_000_000);
