use std::str::FromStr;

use cosmwasm_std::{Deps, Env, Uint128};
use neutron_std::types::cosmos::base::query::v1beta1::PageRequest;
use neutron_std::types::cosmos::base::v1beta1::Coin;
use neutron_std::types::neutron::dex::{tick_liquidity, MsgWithdrawalWithShares, TickLiquidity};
use neutron_std::types::neutron::util::precdec::PrecDec;

use crate::state::{PositionType, Weight, VAULT_STATE};

use crate::error::ContractError;

pub fn max_price() -> PrecDec {
    PrecDec::from_str("2020125331305056766452345").unwrap()
}

/// Converts a price to a tick index.
/// This is used to calculate the tick index for the AMM Deposit.
pub fn price_to_tick_index(price: &PrecDec) -> Result<i64, ContractError> {
    // Ensure the price is greater than 0
    if price.is_zero() || price < &PrecDec::zero() {
        return Err(ContractError::InvalidPrice(*price));
    }

    // Convert PrecDec to f64
    let price_f64 = price.to_string().parse::<f64>().map_err(|_| {
        ContractError::ConversionError("Failed to convert price to f64".to_string())
    })?;

    // Compute the logarithm of the base (1.0001)
    let log_base = 1.0001f64.ln();

    // Compute the logarithm of the price
    let log_price = price_f64.ln();

    // Calculate the tick index using the formula: TickIndex = -log(Price) / log(1.0001)
    let tick_index = (log_price / log_base);

    // Convert the tick index to i64, rounding to the nearest integer
    Ok(tick_index.round() as i64)
}

pub fn sort_token_data_and_get_pair_id_str(token0: &String, token1: &String) -> String {
    let mut tokens = [token0.clone(), token1.clone()];
    if token1 < token0 {
        tokens.reverse();
    }

    tokens.join("<>")
}

pub fn get_tick_index_for_liquidity(liquidity: &TickLiquidity) -> i64 {
    let liq = liquidity.liquidity.as_ref().unwrap();
    match liq {
        tick_liquidity::Liquidity::PoolReserves(pool_reserves) => {
            pool_reserves
                .key
                .as_ref()
                .unwrap()
                .tick_index_taker_to_maker
        }
        tick_liquidity::Liquidity::LimitOrderTranche(limit_order) => {
            limit_order.key.as_ref().unwrap().tick_index_taker_to_maker
        }
    }
}

pub const ONE_ITEM_PAGINATION: PageRequest = PageRequest {
    key: vec![],
    offset: 0,
    limit: 1,
    count_total: false,
    reverse: false,
};

pub fn tick_index_to_price(tick_index: i64) -> PrecDec {
    let price_base = PrecDec::from_str("1.0001").unwrap();
    price_base.pow(tick_index as u32)
}

pub fn calc_shares_proportion(shares: &Coin, liquidity_proportion: &Weight) -> Coin {
    let amount = shares.amount.parse::<Uint128>().unwrap();
    Coin {
        amount: liquidity_proportion.mul_raw(amount).to_string(),
        denom: shares.denom.clone(),
    }
}

pub fn remove_liquidity_msg(
    for_position: PositionType,
    deps: Deps,
    env: &Env,
    liquidity_proportion: &Weight,
) -> Option<MsgWithdrawalWithShares> {
    if liquidity_proportion.is_zero() {
        return None;
    }

    // Invariant: After instantiation, `VAULT_STATE` is always present.
    let position_shares = VAULT_STATE
        .load(deps.storage)
        .unwrap()
        .from_position_type(for_position);

    if let Some(position_shares) = position_shares {
        // Invariant: We know any position liquidity is a valid PrecDec.
        let shares_to_remove: Vec<Coin> = position_shares
            .clone()
            .iter()
            .map(|share| calc_shares_proportion(share, liquidity_proportion))
            .filter_map(|coin| {
                // Filter out zero-amount shares as Go validation requires positive amounts
                // Parse the amount to ensure it's a valid positive number
                match coin.amount.parse::<Uint128>() {
                    Ok(amount) if !amount.is_zero() => Some(coin),
                    _ => None,
                }
            })
            .collect();

        // Only create message if there are shares to remove
        if shares_to_remove.is_empty() {
            return None;
        }

        let contract_addr_str = env.contract.address.to_string();

        Some(MsgWithdrawalWithShares {
            shares_to_remove,
            creator: contract_addr_str.clone(),
            receiver: contract_addr_str,
        })
    } else {
        None
    }
}

pub fn calc_bounded_tick_range(lower_tick: i64, upper_tick: i64, max_ticks: i64) -> (i64, i64) {
    if upper_tick < lower_tick {
     panic!("upper_tick < lower_tick");
    }
    if (upper_tick - lower_tick).abs() <= max_ticks {
        return (lower_tick, upper_tick);
    } else {
        let middle_tick = (upper_tick + lower_tick) / 2;
        return (middle_tick - max_ticks / 2, middle_tick + max_ticks / 2);   
    }
}