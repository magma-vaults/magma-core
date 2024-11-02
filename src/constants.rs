use std::str::FromStr;

use cosmwasm_std::{Addr, Decimal, Uint128};
use once_cell::sync::Lazy;

pub const MIN_TICK: i32 = -108_000_000;
pub const MAX_TICK: i32 = 342_000_000;
pub const MIN_LIQUIDITY: Uint128 = Uint128::new(1000);
pub const TWAP_SECONDS: u64 = 60;
pub static POSITION_CREATION_SLIPPAGE: Lazy<Decimal> = Lazy::new(|| Decimal::from_str("0.997").unwrap());

pub static PROTOCOL: Lazy<Addr> = Lazy::new(|| Addr::unchecked("osmo1a8gd76fw6umx652v7cs73vnge2zju8s8hcm86t"));
pub static DEFAULT_PROTOCOL_FEE: Lazy<Decimal> = Lazy::new(|| Decimal::from_str("0.05").unwrap());
pub static MAX_PROTOCOL_FEE: Lazy<Decimal> = Lazy::new(|| Decimal::from_str("0.1").unwrap());
// FIXME: USDC denom for mainnet.
// pub const VAULT_CREATION_COST_DENOM: &str = "ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4";
// FIXME: USDC denom for testnet.
pub const VAULT_CREATION_COST_DENOM: &str = "ibc/DE6792CF9E521F6AD6E9A4BDF6225C9571A3B74ACC0A529F92BC5122A39D2E58";
// FIXME: 20 USDC max vault creation cost. Its only proper as USDC has 6 decimals.
pub static MAX_VAULT_CREATION_COST: Uint128 = Uint128::new(20_000_000);
// FIXME: Small USDC amount for testing purposes, refactor all this bloat anyways, each vault gets
//        instantiated only once anyways.
pub static DEFAULT_VAULT_CREATION_COST: Uint128 = Uint128::new(1_000);
