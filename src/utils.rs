use std::str::FromStr;
use cosmwasm_std::{Decimal, Decimal256, Int128, SignedDecimal256, Uint128};
use crate::state::{PositiveDecimal, PriceFactor, Weight};

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

#[macro_export]
macro_rules! assert_approx_eq {
    ($a:expr, $b:expr, $tol:expr) => {
        let d = if $a > $b {
            $a - $b
        } else { 
            $b - $a 
        };

        if d > $tol {
            panic!(
                "assertion failed: `abs(left - right) <= tolerance` \
                 (left: `{:?}`, right: `{:?}`, tolerance: `{:?}`)",
                $a, $b, $tol
            );
            
        }
    };
}


pub fn raw<T: From<Uint128>>(d: &Decimal) -> T {
    d.atomics().into()
}

// TODO: Prove downgrade to i32 is safe.
/// Generalized inverse of Osmosis price function. Thus, it will
/// map each price to its closest tick.
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

    // Invariant: Wont overflow/underflow under i256.
    // Proof: I have the proof in a obsidian note, TODO I need to
    //        properly formalize it in doc or a whitepaper.
    compute_price_inverse(p).unwrap()
}

/// # Arguments
///
/// * `k` - Price factor for the base range position.
/// * `w` - Weight for the full range position.
/// * `x` - Amount of token0 to be used for the full range position
///         and the base one. Thus, `y = p*x`.
///
/// # Returns
///
/// The amount of token0 `x0` to use in a full range position for
/// its liquidity to be `w*L`, where `L` is the total liquidity
/// of both, the full range position, and the base one. Read
/// whitepaper for further clarification (TODO).
pub fn calc_x0(k: &PriceFactor, w: &Weight, x: Decimal) -> Decimal {
    if w.is_zero() { return Decimal::zero() }
    // Invariant: Wont overflow.
    // Proof: I have the proof in a obsidian note, TODO I need to
    //        properly formalize it in doc or a whitepaper.
    do_me! {
        let sqrt_k = k.0.sqrt();

        let numerator = w.mul_dec(&sqrt_k);
        let numerator = Decimal256::from(numerator)
            .checked_mul(x.into())?;

        let denominator = sqrt_k
            .checked_sub(Decimal::one())?
            .checked_add(w.0)?;

        let x0 = numerator.checked_div(denominator.into())?;
        Decimal::try_from(x0)?
    }.unwrap()
}

