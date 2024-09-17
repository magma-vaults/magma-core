use std::str::FromStr;
use cosmwasm_std::{Decimal, Int128, SignedDecimal256, Uint128};
use crate::state::PositiveDecimal;

/// Used to chain anyhow::Result computations 
/// without closure boilerplate.
#[macro_export]
macro_rules! do_ok {
    ($($code:tt)*) => {
        (|| -> ::anyhow::Result<_> {
            Ok($($code)*)
        })()
    }
}

/// Used to build do-notation like blocks with anyhow::Result 
/// without closure boilerplate.
#[macro_export]
macro_rules! do_me {
    ($($body:tt)*) => {
        (|| -> ::anyhow::Result<_> {
            Ok({
                $($body)*
            })
        })()
    }
}

pub fn raw<T: From<Uint128>>(d: &Decimal) -> T {
    d.atomics().into()
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
            SignedDecimal256::one()
                .checked_div(ten.checked_pow(exp.unsigned_abs()).ok()?)
                .ok()
        }
    };

    let compute_price_inverse = |p| {
        let floor_log_p = PositiveDecimal::new(p)?.floorlog10();
        let x = floor_log_p.checked_mul(9)?.checked_sub(1)?;

        let x = maybe_neg_pow(floor_log_p)?
            .checked_mul(SignedDecimal256::from_str(&x.to_string()).ok()?)
            .ok()?
            .checked_add(SignedDecimal256::from(*p))
            .ok()?;

        let x = maybe_neg_pow(6i32.checked_sub(floor_log_p)?)?
            .checked_mul(x)
            .ok()?;

        let x: Int128 = x.to_int_floor().try_into().ok()?;
        x.i128().try_into().ok()
    };

    // Invariant: Price function inverse computation doesnt overflow under i256.
    //     Proof: See whitepaper theorem 5.
    compute_price_inverse(p).unwrap()
}
