use neutron_std::types::neutron::dex::{TickLiquidity, tick_liquidity};

  pub fn sort_token_data_and_get_pair_id_str(
    token0: &String,
    token1: &String,
) -> String {
    let mut tokens = [token0.clone(), token1.clone()];
    if token1 < token0 {
        tokens.reverse();
    }
    
    tokens.join("<>")
}

pub fn get_tick_index_for_liquidity(liquidity: &TickLiquidity) -> i64 {
    let liq = liquidity.liquidity.as_ref().unwrap();
    match liq {
        tick_liquidity::Liquidity::PoolReserves(pool_reserves) => pool_reserves.key.as_ref().unwrap().tick_index_taker_to_maker,
        tick_liquidity::Liquidity::LimitOrderTranche(limit_order) => limit_order.key.as_ref().unwrap().tick_index_taker_to_maker,
        _ => panic!("No liquidity found"),
    }
}