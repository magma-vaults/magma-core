use std::str::FromStr;

use cosmwasm_std::{Addr, Decimal};
use once_cell::sync::Lazy;

pub const MIN_TICK: i32 = -108_000_000;
pub const MAX_TICK: i32 = 342_000_000;
pub static PROTOCOL: Lazy<Addr> = Lazy::new(|| Addr::unchecked("TODO"));
pub static MAX_PROTOCOL_FEE: Lazy<Decimal> = Lazy::new(|| Decimal::from_str("0.1").unwrap());
