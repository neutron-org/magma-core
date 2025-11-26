use std::str::FromStr;

use crate::{constants::{MAX_TICK, MIN_TICK}, state::{PriceFactor, Weight}};
use cosmwasm_std::{Uint128, Uint256};
use neutron_std::types::neutron::util::precdec::PrecDec;

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
        let d = if $a > $b { $a - $b } else { $b - $a };

        if d > $tol {
            panic!(
                "assertion failed: `abs(left - right) <= tolerance` \
                 (left: `{:?}`, right: `{:?}`, tolerance: `{:?}`)",
                $a, $b, $tol
            );
        }
    };
}

// TODO: this hides a floor opertation. Should just use Uint256_to_uint128
pub fn uint256_to_uint128(uint256: &Uint256) -> Uint128 {
    // TODO add check that this does not overflow. It really should though
    if uint256.gt(&Uint128::MAX.into()) {
        panic!("uint256_to_uint128: uint256 is greater than Uint128::MAX");
    }
    let uint_str = uint256.to_string();
    Uint128::from_str(&uint_str).unwrap()
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
    if w.is_zero() {
        return PrecDec::zero();
    }
    // Invariant: Wont overflow.
    // Proof: I have the proof in a obsidian note, TODO I need to
    //        properly formalize it in doc or a whitepaper.
    do_me! {
        let w_precdec = w.to_precdec();
        let sqrt_k = k.0.sqrt();

        let numerator = w_precdec.checked_mul(sqrt_k)?;
        let numerator = numerator.checked_mul(x)?;

        let denominator = sqrt_k
            .checked_sub(PrecDec::one())?
            .checked_add(w_precdec)?;

        if denominator.is_zero() {
            panic!("calc_x0: denominator is zero");
        }

        let x0 = numerator.checked_div(denominator.into())?;
        x0
    }
    .unwrap()
}
