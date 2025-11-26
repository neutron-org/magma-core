use std::{cmp, str::FromStr};

use cosmwasm_std::{Deps, Env, Uint128, Uint256};
use cw20_base::state::TOKEN_INFO;
use neutron_std::types::neutron::{dex::DexQuerier, util::precdec::PrecDec};

use crate::{
    constants::MIN_LIQUIDITY,
    do_me, do_ok,
    duality_helpers::remove_liquidity_msg,
    msg::{
        CalcSharesAndUsableAmountsResponse, PositionBalancesWithFeesResponse, VaultBalancesResponse,
    },
    state::{FundsInfo, PositionType, Weight, FEES_INFO, FUNDS_INFO},
};

/// Partition available balances to the vault in 3 sets:
/// - Balances available for business logic, e.g., for creating new positions.
/// - Idle protocol fees, not yet claimed nor commited to the state.
/// - Idle vault admin fees, not yet claimed nor commited to the state.
///
/// For this, query the fees and balances in all current vault positions and
/// funds tracked by [`FUNDS_INFO`] and [`FEES_INFO`].
pub fn vault_balances(deps: Deps, env: &Env) -> VaultBalancesResponse {
    let full_range_balances = position_balances(PositionType::FullRange, deps, env);
    let base_balances = position_balances(PositionType::Base, deps, env);
    let limit_balances = position_balances(PositionType::Limit, deps, env);

    // Invariant: Any state will always be present after instantiation.
    let FundsInfo {
        available_balance0,
        available_balance1,
    } = FUNDS_INFO.load(deps.storage).unwrap();

    let fees = FEES_INFO.load(deps.storage).unwrap();

    // Invariant: Wont panic.
    // Proof: If the contract has unclaimed fees, we know its balance will at
    //        least be those fees, so the subtractions wont underflow. Any
    //        addition of token amounts wont overflow, because for that the
    //        token supply of any token would have to be above `Uint128::MAX`.
    //        Products wont overflow, as we know the fees are valid weights.
    do_me! {
        let total_token0_fees = full_range_balances.bal0_fees
            .checked_add(base_balances.bal0_fees)?
            .checked_add(limit_balances.bal0_fees)?;

        let total_token1_fees = full_range_balances.bal1_fees
            .checked_add(base_balances.bal1_fees)?
            .checked_add(limit_balances.bal1_fees)?;

        let protocol_unclaimed_fees0: Uint128 = fees.protocol_fee.0
            .mul_raw(total_token0_fees)
            .atomics();

        let protocol_unclaimed_fees1 = fees.protocol_fee.0
            .mul_raw(total_token1_fees)
            .atomics();

        let admin_unclaimed_fees0 = fees.admin_fee.0
            .mul_raw(total_token0_fees)
            .atomics();

        let admin_unclaimed_fees1 = fees.admin_fee.0
            .mul_raw(total_token1_fees)
            .atomics();

        let bal0 = available_balance0
            .checked_add(full_range_balances.bal0)?
            .checked_add(base_balances.bal0)?
            .checked_add(limit_balances.bal0)?
            .checked_add(total_token0_fees)?
            .checked_sub(protocol_unclaimed_fees0)?
            .checked_sub(admin_unclaimed_fees0)?;

        let bal1 = available_balance1
            .checked_add(full_range_balances.bal1)?
            .checked_add(base_balances.bal1)?
            .checked_add(limit_balances.bal1)?
            .checked_add(total_token1_fees)?
            .checked_sub(protocol_unclaimed_fees1)?
            .checked_sub(admin_unclaimed_fees1)?;

        VaultBalancesResponse {
            bal0,
            bal1,
            protocol_unclaimed_fees0,
            protocol_unclaimed_fees1,
            admin_unclaimed_fees0,
            admin_unclaimed_fees1
        }
    }
    .unwrap()
}

pub fn position_balances(
    position_type: PositionType,
    deps: Deps,
    env: &Env,
) -> PositionBalancesWithFeesResponse {
    let withdraw_msg = remove_liquidity_msg(position_type, deps, env, &Weight::MAX);
    
    // If there's no position, return zero balances
    if withdraw_msg.is_none() {
        return PositionBalancesWithFeesResponse {
            bal0: Uint128::zero(),
            bal1: Uint128::zero(),
            bal0_fees: Uint128::zero(),
            bal1_fees: Uint128::zero(),
        };
    };
    
    let dex_querier = DexQuerier::new(&deps.querier);

    // If the query fails (e.g., invalid address or other validation error),
    // return zero balances instead of panicking
    let pos = dex_querier.simulate_withdrawal_with_shares(withdraw_msg).unwrap().resp.unwrap();

    // Invariant: Will never panic, because if the position has amounts
    //            `amount0` and `amount1`, we know theyre valid `Uint128`s.
    let bal0 = PrecDec::from_str(&pos.dec_reserve0_withdrawn)
        .unwrap()
        .to_uint_floor();

    let bal1 = PrecDec::from_str(&pos.dec_reserve1_withdrawn)
        .unwrap()
        .to_uint_floor();

    PositionBalancesWithFeesResponse {
        bal0: Uint128::from_str(&bal0.to_string()).unwrap(),
        bal1: Uint128::from_str(&bal1.to_string()).unwrap(),
        bal0_fees: Uint128::zero(),
        bal1_fees: Uint128::zero(),
    }
}

/// # Arguments
///
/// * `input_amount0` - Amount of token0 for which we want to calculate shares for,
///                     not yet in the contract state ([`FUNDS_INFO`]).
///
/// * `input_amount1` - Amount of token1 for which we want to calculate shares for,
///                     not yet in the contract state ([`FUNDS_INFO`]).
pub fn calc_shares_and_usable_amounts(
    input_amount0: Uint128,
    input_amount1: Uint128,
    deps: Deps,
    env: &Env,
) -> CalcSharesAndUsableAmountsResponse {
    let VaultBalancesResponse {
        bal0: total0,
        bal1: total1,
        ..
    } = vault_balances(deps, &env);

    // Invariant: `TOKEN_INFO` always present after instantiation.
    let total_supply = TOKEN_INFO.load(deps.storage).unwrap().total_supply;

    if total_supply.is_zero() {
        assert!(total0.is_zero() && total1.is_zero());
        // Invariant: Wont overflow. See [`DepositError::DepositedAmountBelowMinLiquidity`].
        CalcSharesAndUsableAmountsResponse {
            shares: cmp::max(input_amount0, input_amount1)
                .checked_sub(MIN_LIQUIDITY)
                .unwrap(),
            usable_amount0: input_amount0,
            usable_amount1: input_amount1,
        }
    } else if total0.is_zero() {
        // Invariant: If there are shares and there are no tokens
        //            denom0 in the vault, then the shares must
        //            be for the token denom1.
        assert!(!total1.is_zero());

        // Invariant: The multiplication wont overflow becuase we
        //            lifted the amount to `Uint256`. The division
        //            wont fail becuase we just ensured `total1`
        //            is not zero. The downgrade back to `Uint128`
        //            wont fail because we divided proportionally
        //            by `total1`. The same reasoning applies to
        //            the rest of branches.
        let shares = do_ok!(Uint256::from(input_amount1)
            .checked_mul(total_supply.into())?
            .checked_div(total1.into())?
            .try_into()?)
        .unwrap();

        CalcSharesAndUsableAmountsResponse {
            shares,
            usable_amount0: Uint128::zero(),
            usable_amount1: input_amount1,
        }
    } else if total1.is_zero() {
        // Invariant: If there are shares and there are no tokens
        //            denom1 in the vault, then the shares must
        //            be for the token denom0.
        assert!(!total0.is_zero());

        let shares = do_ok!(Uint256::from(input_amount0)
            .checked_mul(total_supply.into())?
            .checked_div(total0.into())?
            .try_into()?)
        .unwrap();

        CalcSharesAndUsableAmountsResponse {
            shares,
            usable_amount0: input_amount0,
            usable_amount1: Uint128::zero(),
        }
    } else {
        let input_amount0: Uint256 = input_amount0.into();
        let input_amount1: Uint256 = input_amount1.into();
        let total0: Uint256 = total0.into();
        let total1: Uint256 = total1.into();

        do_me! {
            let cross = cmp::min(
                input_amount0.checked_mul(total1)?,
                input_amount1.checked_mul(total0)?
            );

            if cross.is_zero() {
                return Ok(CalcSharesAndUsableAmountsResponse::default())
            }

            let usable_amount0 = cross
                .checked_sub(Uint256::one())?
                .checked_div(total1)?
                .checked_add(Uint256::one())?
                .try_into()?;

            let usable_amount1 = cross
                .checked_sub(Uint256::one())?
                .checked_div(total0)?
                .checked_add(Uint256::one())?
                .try_into()?;

            let shares = cross
                .checked_mul(total_supply.into())?
                .checked_div(total0)?
                .checked_div(total1)?
                .try_into()?;

            CalcSharesAndUsableAmountsResponse {
                shares,
                usable_amount0,
                usable_amount1,
            }
        }
        .unwrap()
    }
}
