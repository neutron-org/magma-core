use crate::constants::{
    MAX_PROTOCOL_FEE, MAX_TICK, MAX_VAULT_CREATION_COST, TWAP_SECONDS, VAULT_CREATION_COST_DENOM
};
use crate::error::{DexError, InstantiationError, ProtocolOperationError};
use crate::{
    constants::MIN_TICK,
    msg::{VaultInfoInstantiateMsg, VaultParametersInstantiateMsg, VaultRebalancerInstantiateMsg},
};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Deps, Env, MessageInfo, QuerierWrapper, Timestamp, Uint128};
use cw_storage_plus::Item;
// use osmosis_std::types::osmosis::twap::v1beta1::TwapQuerier;
use osmosis_std::types::osmosis::{
    concentratedliquidity::v1beta1::Pool, poolmanager::v1beta1::PoolmanagerQuerier,
};
use neutron_std::types::{neutron::dex::DexQuerier,
cosmos::base::v1beta1::{Coin},
neutron::util::precdec::PrecDec
};
use readonly;
use std::{cmp::min_by_key, str::FromStr};
use crate::duality_helpers::{ONE_ITEM_PAGINATION, tick_index_to_price, get_tick_index_for_liquidity};

#[cw_serde]
#[readonly::make]
pub struct Weight(pub PrecDec);
impl Weight {
    pub const MAX: PrecDec = PrecDec::one();

    pub fn new(value: &str) -> Option<Self> {
        let value = PrecDec::from_str(value).ok()?;
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
        Self(PrecDec::zero())
    }

    pub fn max() -> Self {
        Self(Self::MAX)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == PrecDec::zero()
    }

    pub fn is_max(&self) -> bool {
        self.0 == Weight::MAX
    }
}

impl TryFrom<PrecDec> for Weight {
    type Error = ();
    fn try_from(value: PrecDec) -> Result<Self, Self::Error> {
        if value > Self::MAX {
            Err(())
        } else {
            Ok(Self(value))
        }
    }
}

#[cw_serde]
#[readonly::make]
pub struct PositiveDecimal(pub PrecDec);
impl PositiveDecimal {
    pub fn new(value: &PrecDec) -> Option<Self> {
        (value != PrecDec::zero()).then_some(Self(*value))
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

    pub fn current_tick0(&self, querier: &QuerierWrapper) -> Result<i64, DexError> {
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
    pub fn price(&self, querier: &QuerierWrapper) -> Result<Decimal, DexError> {

        let price_tick = self.current_tick0(querier)?;
        let price = tick_index_to_price(price_tick);
        Ok(price)
    }
}

#[cw_serde]
#[readonly::make]
pub struct PriceFactor(pub Decimal);
impl PriceFactor {
    pub fn new(value: &str) -> Option<Self> {
        let value = Decimal::from_str(value).ok()?;
        (value >= Decimal::one()).then_some(Self(value))
    }

    pub fn is_one(&self) -> bool {
        self.0 == Decimal::one()
    }
}

#[cw_serde]
#[readonly::make]
pub struct ProtocolFee(pub Weight);
impl ProtocolFee {
    pub fn max() -> Decimal {
        *MAX_PROTOCOL_FEE
    }

    pub fn new(value: &str) -> Option<Self> {
        let value = Weight::new(value)?;
        (value.0 <= Self::max()).then_some(Self(value))
    }

    pub fn zero() -> ProtocolFee {
        Self(Weight::zero())
    }
}

impl Default for ProtocolFee {
    fn default() -> Self {
        // Invariant: Wont panic, `Self::max()` is 0.1,
        Self::new("0.05").unwrap()
    }
}

#[cw_serde]
#[readonly::make]
pub struct VaultCreationCost(pub Uint128);
impl VaultCreationCost {
    pub fn max() -> Uint128 {
        MAX_VAULT_CREATION_COST
    }

    pub fn new(value: Uint128) -> Option<Self> {
        (value <= Self::max()).then_some(Self(value))
    }
}

impl Default for VaultCreationCost {
    fn default() -> Self {
        // Invariant: Wont panic, `Self::max()` is 20_000_000.
        // FIXME: Really low default for testing purposes.
        Self::new(Uint128::new(1_000)).unwrap()
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
    /// then the vault wont have aa limit order, and will just hold remaining
    /// tokens.
    pub limit_factor: PriceFactor,
    /// Decimal weight, zero if we dont want a full range position.
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

        // NOTE: We dont support vaults with idle capital nor less than 3 positions for now.
        //       Integrating both options is trivial, but we keep it simple for the v1.
        match (
            full_range_weight.is_zero(),
            base_factor.is_one(),
            limit_factor.is_one(),
        ) {
            (false, false, false) => Ok(()),
            (true, true, true) => Err(ContradictoryConfig {
                reason:
                    "All vault parameters will produce null positions, all capital would be idle"
                        .into(),
            }),
            (true, true, _) => Err(ContradictoryConfig {
                reason: "A vault without balanced orders will have idle capital".into(),
            }),
            (_, _, true) => Err(ContradictoryConfig {
                reason: "A vault without a limit order will have idle capital".into(),
            }),
            (_, true, _) if !full_range_weight.is_max() => Err(ContradictoryConfig {
                reason: "If the vault doenst have a base order, the full range weight should be 1"
                    .into(),
            }),
            (_, false, _) if full_range_weight.is_max() => Err(ContradictoryConfig {
                reason: "If the full range weight is 1, the base factor should also be".into(),
            }),
            _ => Err(ContradictoryConfig {
                reason: "We dont support vaults with less than 3 positions for now".into(),
            }),
        }?;

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
    pub rebalancer: VaultRebalancer,
}

impl VaultInfo {
    pub fn new(info: VaultInfoInstantiateMsg, deps: Deps) -> Result<Self, InstantiationError> {
        use InstantiationError::*;
        let pool_id =
            PoolId::new(info.pool_id, &deps.querier).ok_or(InvalidPoolId(info.pool_id))?;

        assert!(pool_id.0 == info.pool_id);

        let rebalancer = VaultRebalancer::new(info.rebalancer, deps)?;

        let admin = if let Some(admin) = info.admin {
            Some(
                deps.api
                    .addr_validate(&admin)
                    .map_err(|_| InvalidAdminAddress(admin))?,
            )
        } else {
            match rebalancer {
                VaultRebalancer::Anyone { .. } => Ok(None),
                _ => Err(ContradictoryConfig {
                    reason: "If admin is none, the rebalancer can only be anyone".into(),
                }),
            }?
        };

        Ok(VaultInfo {
            pair_id: pool_id,
            rebalancer,
            admin,
        })
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

    pub fn current_tick(&self, querier: &QuerierWrapper) -> i64 {
        // Invariant: Wont panic as max and min possible ticks below 2**31 - 1.
        self.pair_id.current_tick0(querier).unwrap()
    }

    /// Min possible tick 
    pub fn min_valid_tick(&self) -> i64 {
            -559680
    }   
    
    /// Max possible tick 
    pub fn max_valid_tick(&self) -> i64 {
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
                seconds_before_rabalance,
                price_factor_before_rebalance,
            } => Ok(Self::Anyone {
                price_factor_before_rebalance: PriceFactor::new(&price_factor_before_rebalance)
                    .ok_or(InvalidPriceFactor(price_factor_before_rebalance))?,
                time_before_rabalance: Timestamp::from_seconds(seconds_before_rabalance),
            }),
        }
    }
}

#[cw_serde]
pub enum PositionType {
    FullRange,
    Base,
    Limit,
}

type MaybePositionShares = Option<Vec<Coin>>;

// TODO: The bind can be stricter, as the second field can only change
//       in one direction.
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
    pub full_range_position_shares: MaybePositionShares,
    pub base_position_shares: MaybePositionShares,
    pub limit_position_shares: MaybePositionShares,

    /// last price and last timestamp since the last rebalance. Optional as it
    /// requires a first rebalance to happen to be set. After that, both will
    /// always be set.
    pub last_price_and_timestamp: Option<StateSnapshot>,
}

impl VaultState {
    pub fn from_position_type(&self, position_type: PositionType) -> &MaybePositionShares {
        match position_type {
            PositionType::FullRange => &self.full_range_position_shares,
            PositionType::Base => &self.base_position_shares,
            PositionType::Limit => &self.limit_position_shares,
        }
    }
}

#[cw_serde]
#[derive(Default)]
pub struct FeesInfo {
    pub protocol_fee: ProtocolFee,
    pub protocol_tokens0_owned: Uint128,
    pub protocol_tokens1_owned: Uint128,
    pub protocol_vault_creation_cost: VaultCreationCost,
    pub protocol_vault_creation_tokens_owned: Uint128,
    pub admin_fee: ProtocolFee,
    pub admin_tokens0_owned: Uint128,
    pub admin_tokens1_owned: Uint128,
}

impl FeesInfo {
    
    fn validate_vault_creation_cost(info: &MessageInfo) -> Result<Uint128, InstantiationError> {
        let vault_creation_cost = VaultCreationCost::default();

        let paid_amount = cw_utils::must_pay(info, VAULT_CREATION_COST_DENOM).unwrap_or_default();

        if paid_amount != vault_creation_cost.0 {
            Err(InstantiationError::VaultCreationCostNotPaid {
                cost: vault_creation_cost.0.into(),
                denom: VAULT_CREATION_COST_DENOM.into(),
                got: paid_amount.into(),
            })
        } else { Ok(paid_amount) }
    }

    fn validate_admin_fee(admin_fee: String, vault_info: &VaultInfo) -> Result<ProtocolFee, InstantiationError> {
        let admin_fee =
            ProtocolFee::new(&admin_fee).ok_or(InstantiationError::InvalidAdminFee {
                max: ProtocolFee::max().to_string(),
                got: admin_fee,
            })?;

        if !admin_fee.0.is_zero() && vault_info.admin.is_none() {
            Err(InstantiationError::AdminFeeWithoutAdmin {})
        } else { Ok(admin_fee) }
    }

    pub fn new(
        admin_fee: String,
        vault_info: &VaultInfo,
        info: &MessageInfo,
    ) -> Result<FeesInfo, InstantiationError> {
        let paid_amount = Self::validate_vault_creation_cost(info)?;
        let admin_fee = Self::validate_admin_fee(admin_fee, vault_info)?;

        Ok(FeesInfo {
            admin_fee,
            protocol_vault_creation_tokens_owned: paid_amount,
            ..FeesInfo::default()
        })
    }

    pub fn update_admin_fee(&self, admin_fee: String, deps: Deps) -> Result<FeesInfo, InstantiationError> {
        // Invariant: Any state is present after instantitation.
        let vault_info = VAULT_INFO.load(deps.storage).unwrap();
        let admin_fee = Self::validate_admin_fee(admin_fee, &vault_info)?;
        Ok(FeesInfo { admin_fee, ..self.clone() })
    }

    pub fn update_protocol_fee(&self, protocol_fee: String) -> Result<FeesInfo, ProtocolOperationError> {
        let protocol_fee = 
            ProtocolFee::new(&protocol_fee).ok_or(ProtocolOperationError::InvalidProtocolFee { 
                max: (*MAX_PROTOCOL_FEE).to_string(), 
                got: protocol_fee
            })?;

        Ok(FeesInfo { protocol_fee, ..self.clone() })
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

pub const FEES_INFO: Item<FeesInfo> = Item::new("fees_info");

/// FUNDS_INFO Refers to the known funds available to the contract,
/// without counting protocol/admin fees.
pub const FUNDS_INFO: Item<FundsInfo> = Item::new("funds_info");
