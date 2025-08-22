use std::str::FromStr;

use neutron_std::types::cosmos::base::query::v1beta1::PageRequest;
use neutron_std::types::neutron::dex::{tick_liquidity, TickLiquidity};
use neutron_std::types::neutron::util::precdec::PrecDec;

use crate::error::ContractError;


pub fn max_price() -> PrecDec {
    PrecDec::from_str("2020125331305056766452345").unwrap()
}

/// Converts a price to a tick index.
/// This is used to calculate the tick index for the AMM Deposit.
pub fn price_to_tick_index(price: PrecDec) -> Result<i64, ContractError> {
    // Ensure the price is greater than 0
    if price.is_zero() || price < PrecDec::zero() {
        return Err(ContractError::InvalidPrice);
    }

    // Convert PrecDec to f64
    let price_f64 = price
        .to_string()
        .parse::<f64>()
        .map_err(|_| ContractError::ConversionError)?;

    // Compute the logarithm of the base (1.0001)
    let log_base = 1.0001f64.ln();

    // Compute the logarithm of the price
    let log_price = price_f64.ln();

    // Calculate the tick index using the formula: TickIndex = -log(Price) / log(1.0001)
    let tick_index = -(log_price / log_base);

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
        _ => panic!("No liquidity found"),
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
