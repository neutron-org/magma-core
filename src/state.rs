use crate::constants::{DEFAULT_PROTOCOL_FEE, MAX_PROTOCOL_FEE, MAX_TICK, TWAP_SECONDS};
use crate::do_some;
use crate::duality_helpers::{
    get_tick_index_for_liquidity, tick_index_to_price, ONE_ITEM_PAGINATION,
};
use crate::error::DexError;
use crate::error::{InstantiationError, ProtocolOperationError};
use crate::{
    constants::MIN_TICK,
    msg::{VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg},
};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Deps, Env, QuerierWrapper, Timestamp, Uint128};
use cw_storage_plus::Item;
use neutron_std::types::neutron::dex::{
    DexQuerier, MsgDeposit, QueryAllTickLiquidityResponse, TickLiquidity,
};
use neutron_std::types::neutron::util::precdec::PrecDec;

use readonly;
use std::i32;
use std::{cmp::min_by_key, str::FromStr};

#[cw_serde]
#[readonly::make]
pub struct Weight(pub PrecDec);
impl Weight {
    pub const MAX: PrecDec = PrecDec::one();

    pub fn new(value: &Uint128) -> Option<Self> {
        let value = PrecDec::raw(value.u128());
        (value <= Self::MAX).then_some(Self(value))
    }

    pub fn permille(value: u32) -> Option<Self> {
        let value = PrecDec::permille(value);
        (value <= Self::MAX).then_some(Self(value))
    }

    pub fn mul_dec(&self, value: &PrecDec) -> PrecDec {
        // Invariant: A weight product wont ever overflow.
        value.checked_mul(self.0).unwrap()
    }

    pub fn mul_raw(&self, value: Uint128) -> PrecDec {
        self.mul_dec(&PrecDec::raw(value.into()))
    }

    pub fn zero() -> Self {
        Self(Decimal::zero())
    }

    pub fn max() -> Self {
        Self(Self::MAX)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == Decimal::zero()
    }

    pub fn is_max(&self) -> bool {
        self.0 == Weight::MAX
    }
}

impl TryFrom<Decimal> for Weight {
    type Error = ();
    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        Self::new(&value.atomics()).ok_or(())
    }
}

#[cw_serde]
#[readonly::make]
pub struct PositiveDecimal(pub Decimal);
impl PositiveDecimal {
    pub fn new(value: &Decimal) -> Option<Self> {
        (value != Decimal::zero()).then_some(Self(*value))
    }

    pub fn floorlog10(&self) -> i32 {
        let x: u128 = self.0.atomics().into();
        // Invariant: `u128::ilog10(u128::MAX)` fits in `i32`.
        let x: i32 = x.ilog10().try_into().unwrap();
        // Invariant: `ilog10(1) - 18 = 0 - 18` fits in `i32`.
        let x = x.checked_sub(18).unwrap();
        // Invariant: `floor(log10(u128::MAX)) - 18 =  20` and
        //            `floor(log10(1))         - 18 = -18`
        assert!((-18..=20).contains(&x));
        x
    }
}

#[cw_serde]
#[readonly::make]
pub struct PairId(pub [String; 2]);
impl PairId {
    pub fn new(pair_id: [String; 2]) -> Option<Self> {
        let mut pair_id_sorted = pair_id.clone();
        pair_id_sorted.sort();

        Some(Self(pair_id_sorted))
    }

    pub fn pair_id_str(&self) -> String {
        self.0.join("<>")
    }

    pub fn current_tick0(&self, querier: &QuerierWrapper) -> Result<i32, DexError> {
        let querier = DexQuerier::new(querier);
        let pair_id_str = self.pair_id_str();

        let liq_token0 = querier.tick_liquidity_all(
            pair_id_str.clone(),
            self.0[0].clone(),
            Some(ONE_ITEM_PAGINATION),
        );
        let liq_token1 = querier.tick_liquidity_all(
            pair_id_str.clone(),
            self.0[1].clone(),
            Some(ONE_ITEM_PAGINATION),
        );

        if liq_token0.is_err() && liq_token1.is_err() {
            return Err(DexError::CannotFetchPrice());
        }

        let price_tick: i64;

        if liq_token0.is_err() {
            price_tick = get_tick_index_for_liquidity(&liq_token1.unwrap().tick_liquidity[0]) * -1;
        } else if liq_token1.is_err() {
            price_tick = get_tick_index_for_liquidity(&liq_token0.unwrap().tick_liquidity[0]);
        } else {
            let price_tick0 = get_tick_index_for_liquidity(&liq_token0.unwrap().tick_liquidity[0]);
            let price_tick1 = get_tick_index_for_liquidity(&liq_token1.unwrap().tick_liquidity[0]);
            price_tick = (price_tick0 + price_tick1) / 2;
        }
        Ok(price_tick)
    }
    // Returns price of token0 denominated in token1
    pub fn price0(&self, querier: &QuerierWrapper) -> Result<PrecDec, DexError> {

        let price_tick = self.current_tick0(querier)?;
        Ok(tick_index_to_price(price_tick))
    }
}

// pub fn twap(&self, querier: &QuerierWrapper, env: &Env) -> Option<Decimal> {
//     let start_time = env.block.time;
//     // Invariant: Wont overflow as `env.block.time` is reasonable.
//     let osmosis_start_time = Some(osmosis_std::shim::Timestamp {
//         seconds: start_time.seconds().saturating_sub(TWAP_SECONDS).try_into().unwrap(),
//         nanos: 0
//     });
//     let pool = self.to_pool(querier);

//     // Invariant: Will only return `None` if `pool` was recently created, as
//     //            we already ensured that `self` is valid during instantiation
//     //            and that the start time is in the near past.
//     let p = TwapQuerier::new(querier)
//         .geometric_twap_to_now(self.0, pool.token0, pool.token1, osmosis_start_time)
//         .ok()?
//         .geometric_twap;

//     // Invariant: We know `.geometric_twap_to_now(...)` returns valid `Decimal` values.
//     Some(Decimal::from_str(&p).unwrap())
// }

#[cw_serde]
#[readonly::make]
pub struct PriceFactor(pub PrecDec);
impl PriceFactor {
    pub fn new(value: &Uint128) -> Option<Self> {
        let value = PrecDec::from_str(&value.to_string()).unwrap();
        (value >= PrecDec::from_str("1.0001").unwrap()).then_some(Self(value))
    }

    pub fn is_one(&self) -> bool {
        self.0 == PrecDec::one()
    }
}

#[cw_serde]
#[readonly::make]
pub struct ProtocolFee(pub Weight);
impl ProtocolFee {
    pub fn max() -> Decimal {
        MAX_PROTOCOL_FEE
    }

    pub fn new(value: &Uint128) -> Option<Self> {
        let value = Weight::new(value)?;
        (value.0 <= Self::max()).then_some(Self(value))
    }

    pub fn zero() -> ProtocolFee {
        Self(Weight::zero())
    }
}

impl Default for ProtocolFee {
    fn default() -> Self {
        // Invariant: Wont panic as the const is in [0, 1].
        Self(Weight::try_from(DEFAULT_PROTOCOL_FEE).unwrap())
    }
}

#[cw_serde]
pub struct VaultParameters {
    /// Price factor for the base order. Thus, if the current price is `p`,
    /// then the base position will have range `[p/base_factor, p*base_factor]`.
    /// if `base_factor == PriceFactor(Decimal::one())`, then the vault wont
    /// have a base order.
    pub base_factor: PriceFactor,
    /// Price factor for the limit order. Thus, if the current price is `p`,
    /// then the limit position will have either range `[p/limit_factor, p]` or
    /// `[p, p*limit_factor]`. If `limit_factor == PriceFactor(Decimal::one())`,
    /// then the vault wont have a limit order, and will just hold remaining
    /// tokens.
    pub limit_factor: PriceFactor,
    /// Exact liquidity weight to put into the full range order.
    /// Zero if we dont want a full range position.
    pub full_range_weight: Weight,
}

impl VaultParameters {
    pub fn new(params: VaultParametersInstantiateMsg) -> Result<Self, InstantiationError> {
        use InstantiationError::*;
        let base_factor =
            PriceFactor::new(&params.base_factor).ok_or(InvalidPriceFactor(params.base_factor))?;

        let limit_factor = PriceFactor::new(&params.limit_factor)
            .ok_or(InvalidPriceFactor(params.limit_factor))?;

        let full_range_weight = Weight::new(&params.full_range_weight)
            .ok_or(InvalidWeight(params.full_range_weight))?;

        if full_range_weight.is_max() && !base_factor.is_one() {
            return Err(ContradictoryConfig {
                reason: "Allocating all liquidity into the full range implies the vault wont have any base one".into(),
                hint: "Set base_factor to 1 to specify the vault will only manage a full range position".into()
            });
        }

        if !full_range_weight.is_max() && base_factor.is_one() {
            return Err(ContradictoryConfig {
                reason:
                    "A vault without a base order should allocate all liquidity into the full range"
                        .into(),
                hint: "If base_factor is 1, the full_range_weight should also be".into(),
            });
        }

        if limit_factor.is_one() {
            return Err(ContradictoryConfig {
                reason: "A vault without limit positions will generally have idle capital".into(),
                hint: "Set a limit_factor different from 1".into(),
            });
        }

        // Invariant: Those 3 conditions above are enough to ensure the vault doesnt have idle capital.
        // Proof outline: Specify all conditions that produce idle capital and simplify.

        Ok(VaultParameters {
            base_factor,
            limit_factor,
            full_range_weight,
        })
    }
}

#[cw_serde]
#[readonly::make]
pub struct VaultInfo {
    #[readonly]
    pub pair_id: PairId,
    pub admin: Option<Addr>,
    pub proposed_new_admin: Option<Addr>,
    pub rebalancer: VaultRebalancer,
}

impl VaultInfo {
    pub fn new(info: VaultInfoInstantiateMsg, deps: Deps) -> Result<Self, InstantiationError> {
        use InstantiationError::*;
        let pair_id = PairId::new(info.pair_id).ok_or(InvalidPairId(info.pair_id))?;

        let rebalancer = VaultRebalancer::new(info.rebalancer, deps)?;

        let admin = if let Some(admin) = info.admin {
            Some(
                deps.api
                    .addr_validate(&admin)
                    .map_err(|_| InvalidAdminAddress(admin))?,
            )
        } else {
            None
        };

        rebalancer.rebalancer_consistent_with_admin(&admin)?;

        Ok(VaultInfo {
            pair_id,
            rebalancer,
            admin,
            proposed_new_admin: None,
        })
    }

    pub fn propose_new_admin(self, new_admin: String, deps: Deps) -> Option<Self> {
        let proposed_new_admin = Some(deps.api.addr_validate(&new_admin).ok()?);
        Some(Self {
            proposed_new_admin,
            ..self
        })
    }

    pub fn unset_proposed_new_admin(self) -> Self {
        Self {
            proposed_new_admin: None,
            ..self
        }
    }

    pub fn confirm_new_admin(self) -> Self {
        let admin = self.proposed_new_admin;
        Self {
            admin,
            proposed_new_admin: None,
            ..self
        }
    }

    pub fn burn_admin(self) -> Self {
        Self {
            admin: None,
            ..self
        }
    }

    pub fn change_rebalancer(
        self,
        new_rebalancer: VaultRebalancerInstantiateMsg,
        deps: Deps,
    ) -> Result<Self, InstantiationError> {
        let rebalancer = VaultRebalancer::new(new_rebalancer, deps)?;
        rebalancer.rebalancer_consistent_with_admin(&self.admin)?;
        Ok(Self { rebalancer, ..self })
    }

    pub fn demon0(&self) -> String {
        self.pair_id.0[0].clone()
    }

    pub fn demon1(&self) -> String {
        self.pair_id.0[1].clone()
    }

    pub fn denoms(&self) -> (String, String) {
        (self.demon0(), self.demon1())
    }

    pub fn current_tick(&self, querier: &QuerierWrapper) -> i32 {
        // TODO: direction?
        self.pair_id.current_tick0(querier).unwrap()
    }

    

    /// Min possible tick 
    pub fn min_valid_tick(&self) -> i32 {
        -559680
    }

    /// Max possible tick 
    pub fn max_valid_tick(&self) -> i32 {
        559680
    }

    
}

/// See [`VaultRebalancerInstantiateMsg`].
#[cw_serde]
pub enum VaultRebalancer {
    Admin {},
    Delegate {
        rebalancer: Addr,
    },
    Anyone {
        price_factor_before_rebalance: PriceFactor,
        time_before_rabalance: Timestamp,
    },
}

impl VaultRebalancer {
    pub fn new(
        rebalancer: VaultRebalancerInstantiateMsg,
        deps: Deps,
    ) -> Result<Self, InstantiationError> {
        use InstantiationError::*;
        use VaultRebalancerInstantiateMsg::*;

        match rebalancer {
            Delegate { rebalancer } => {
                let rebalancer = deps
                    .api
                    .addr_validate(&rebalancer)
                    .map_err(|_| InvalidDelegateAddress(rebalancer))?;
                Ok(Self::Delegate { rebalancer })
            }
            Admin {} => Ok(Self::Admin {}),
            Anyone {
                seconds_before_rebalance,
                price_factor_before_rebalance,
            } => Ok(Self::Anyone {
                price_factor_before_rebalance: PriceFactor::new(&price_factor_before_rebalance)
                    .ok_or(InvalidPriceFactor(price_factor_before_rebalance))?,
                time_before_rabalance: Timestamp::from_seconds(seconds_before_rebalance.into()),
            }),
        }
    }

    fn rebalancer_consistent_with_admin(
        &self,
        current_vault_admin: &Option<Addr>,
    ) -> Result<(), InstantiationError> {
        if current_vault_admin.is_none() {
            match self {
                VaultRebalancer::Anyone { .. } => Ok(()),
                _ => Err(InstantiationError::ContradictoryConfig {
                    reason: "If admin is none, the rebalancer can only be anyone".into(),
                    hint: "Set an admin or set the rebalancer to anyone".into(),
                }),
            }
        } else {
            Ok(())
        }
    }
}

#[cw_serde]
pub enum PositionType {
    FullRange,
    Base,
    Limit,
}

type MaybePositionId = Option<u64>;

#[cw_serde]
pub struct StateSnapshot {
    pub last_price: PrecDec,
    pub last_timestamp: Timestamp,
}

#[cw_serde]
#[derive(Default)]
pub struct VaultState {
    /// Position Ids are optional because:
    /// 1. Positions are only created on rebalances.
    /// 2. If any of the vault positions is null, then those should
    ///    be `None`, see [`VaultParameters`].
    pub full_range_position_id: MaybePositionId,
    pub base_position_id: MaybePositionId,
    pub limit_position_id: MaybePositionId,

    /// last price and last timestamp since the last rebalance. Optional as it
    /// requires a first rebalance to happen to be set. After that, both will
    /// always be set.
    pub last_price_and_timestamp: Option<StateSnapshot>,
}

impl VaultState {
    pub fn from_position_type(&self, position_type: PositionType) -> MaybePositionId {
        match position_type {
            PositionType::FullRange => self.full_range_position_id,
            PositionType::Base => self.base_position_id,
            PositionType::Limit => self.limit_position_id,
        }
    }
}

#[cw_serde]
#[derive(Default)]
pub struct FeesInfo {
    pub protocol_fee: ProtocolFee,
    pub protocol_tokens0_owned: Uint128,
    pub protocol_tokens1_owned: Uint128,
    pub admin_fee: ProtocolFee,
    pub admin_tokens0_owned: Uint128,
    pub admin_tokens1_owned: Uint128,
}

impl FeesInfo {
    fn validate_admin_fee(
        admin_fee: Uint128,
        vault_info: &VaultInfo,
    ) -> Result<ProtocolFee, InstantiationError> {
        let admin_fee =
            ProtocolFee::new(&admin_fee).ok_or(InstantiationError::InvalidAdminFee {
                max: ProtocolFee::max().atomics(),
                got: admin_fee,
            })?;

        if !admin_fee.0.is_zero() && vault_info.admin.is_none() {
            Err(InstantiationError::AdminFeeWithoutAdmin {})
        } else {
            Ok(admin_fee)
        }
    }

    pub fn new(admin_fee: Uint128, vault_info: &VaultInfo) -> Result<FeesInfo, InstantiationError> {
        let admin_fee = Self::validate_admin_fee(admin_fee, vault_info)?;
        Ok(FeesInfo {
            admin_fee,
            ..FeesInfo::default()
        })
    }

    pub fn update_admin_fee(
        &self,
        admin_fee: Uint128,
        deps: Deps,
    ) -> Result<FeesInfo, InstantiationError> {
        // Invariant: Any state is present after instantitation.
        let vault_info = VAULT_INFO.load(deps.storage).unwrap();
        let admin_fee = Self::validate_admin_fee(admin_fee, &vault_info)?;
        Ok(FeesInfo {
            admin_fee,
            ..self.clone()
        })
    }

    pub fn update_protocol_fee(
        &self,
        protocol_fee: Uint128,
    ) -> Result<FeesInfo, ProtocolOperationError> {
        let protocol_fee =
            ProtocolFee::new(&protocol_fee).ok_or(ProtocolOperationError::InvalidProtocolFee {
                max: MAX_PROTOCOL_FEE.atomics(),
                got: protocol_fee,
            })?;

        Ok(FeesInfo {
            protocol_fee,
            ..self.clone()
        })
    }
}

#[cw_serde]
#[derive(Default)]
pub struct FundsInfo {
    pub available_balance0: Uint128,
    pub available_balance1: Uint128,
}

/// VAULT_INFO Holds non-mathematical generally immutable information
/// about the vault. Its generally immutable as in it can only be
/// changed by the vault admin, but its state cant be changed with
/// any business logic.
pub const VAULT_INFO: Item<VaultInfo> = Item::new("vault_info");

/// VAULT_PARAMETERS Holds mathematical generally immutable information
/// about the vault. Its generally immutable as in it can only be
/// changed by the vault admin, but its state cant be changed with
/// any business logic.
pub const VAULT_PARAMETERS: Item<VaultParameters> = Item::new("vault_parameters");

/// VAULT_STATE Holds any vault state that can and will be changed
/// with contract business logic.
pub const VAULT_STATE: Item<VaultState> = Item::new("vault_state");

/// FEES_INFO Holds any uncollected admin/protocol fees and fee parameters.
pub const FEES_INFO: Item<FeesInfo> = Item::new("fees_info");

/// FUNDS_INFO Refers to the known funds available to the contract,
/// without counting protocol/admin fees.
pub const FUNDS_INFO: Item<FundsInfo> = Item::new("funds_info");
