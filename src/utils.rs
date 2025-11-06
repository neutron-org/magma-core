use std::str::FromStr;

use cosmwasm_std::{PrecDec, Uint128};
use crate::state::{PriceFactor, Weight};

/// Used to chain anyhow::Result computations without closure boilerplate.
#[macro_export]
macro_rules! do_ok {
    ($($code:tt)*) => {
        (|| -> ::anyhow::Result<_> {
            Ok($($code)*)
        })()
    }
}

/// Used to chain Optional computations without closure boilerplate.
#[macro_export]
macro_rules! do_some {
    ($($code:tt)*) => {
        (|| -> Option<_> {
            Some($($code)*)
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

pub fn raw<T: From<Uint128>>(d: &PrecDec) -> T {
    d.atomics().into()
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
pub fn calc_x0(k: &PriceFactor, w: &Weight, x: PrecDec) -> PrecDec {
    if w.is_zero() { return PrecDec::zero() }
    // Invariant: Wont overflow.
    // Proof: I have the proof in a obsidian note, TODO I need to
    //        properly formalize it in doc or a whitepaper.
    do_me! {
        let sqrt_k = k.0.sqrt();

        let numerator = w.mul_dec(&sqrt_k);
        let numerator = PrecDec::from(numerator)
            .checked_mul(x.into())?;

        let denominator = sqrt_k
            .checked_sub(PrecDec::one())?
            .checked_add(w.0)?;

        let x0 = numerator.checked_div(denominator.into())?;
        PrecDec::try_from(x0)?
    }.unwrap()
}

