use alloy::primitives::U256;
use broadcaster_core::query_rpc_pool::QueryRpcPool;
use eyre::{Result, WrapErr};

use super::runtime::public_chain_runtime_config;
use super::types::{
    PublicActionGasFeeQuote, PublicActionGasFeeSelection, PublicActionProgressStep,
};
use crate::settings::EffectiveChainConfig;
use crate::{
    GAS_LIMIT_BUFFER, HttpContext, SelfBroadcastTipFallback, query_rpc_pool_with_http_client,
    resolve_self_broadcast_gas_fee, self_broadcast_gas_fee_quote_from_rpc_pool_with_tip_fallback,
};

pub(super) const PUBLIC_NATIVE_SEND_GAS_UNITS: u64 = 21_000;
pub(super) const PUBLIC_NATIVE_WRAP_GAS_UNITS: u64 = 50_000;
pub(super) const PUBLIC_NATIVE_APPROVE_GAS_UNITS: u64 = 65_000;
pub(super) const PUBLIC_NATIVE_SHIELD_GAS_UNITS: u64 = 650_000;
const PUBLIC_ACTION_BNB_CHAIN_ID: u64 = 56;

#[must_use]
pub fn public_native_action_gas_units(steps: &[PublicActionProgressStep]) -> u64 {
    public_native_action_gas_units_with_buffer(steps, GAS_LIMIT_BUFFER)
}

#[must_use]
pub(super) fn public_native_action_gas_units_with_buffer(
    steps: &[PublicActionProgressStep],
    gas_limit_buffer: u64,
) -> u64 {
    steps.iter().fold(0_u64, |total, step| {
        let gas_units = public_native_step_gas_units(*step);
        if gas_units == 0 {
            total
        } else {
            total.saturating_add(gas_units + gas_limit_buffer)
        }
    })
}

#[must_use]
pub fn public_native_action_gas_reserve(
    max_fee_per_gas: u128,
    steps: &[PublicActionProgressStep],
) -> U256 {
    public_native_action_gas_reserve_with_buffer(max_fee_per_gas, steps, GAS_LIMIT_BUFFER)
}

#[must_use]
fn public_native_action_gas_reserve_with_buffer(
    max_fee_per_gas: u128,
    steps: &[PublicActionProgressStep],
    gas_limit_buffer: u64,
) -> U256 {
    U256::from(public_native_action_gas_units_with_buffer(
        steps,
        gas_limit_buffer,
    )) * U256::from(max_fee_per_gas)
}

pub async fn quote_public_action_gas_fee(
    chain_id: u64,
    effective_chain: Option<&EffectiveChainConfig>,
    http: &HttpContext,
) -> Result<PublicActionGasFeeQuote> {
    let chain = public_chain_runtime_config(chain_id, effective_chain)?;
    let query_rpc_pool = query_rpc_pool_with_http_client(chain.rpc_urls, http);
    public_action_gas_fee_quote_from_rpc_pool(&query_rpc_pool, http.network_mode(), chain_id).await
}

pub async fn estimate_public_native_action_gas_reserve(
    chain_id: u64,
    steps: &[PublicActionProgressStep],
    effective_chain: Option<&EffectiveChainConfig>,
    gas_fee: PublicActionGasFeeSelection,
    http: &HttpContext,
) -> Result<U256> {
    let chain = public_chain_runtime_config(chain_id, effective_chain)?;
    let query_rpc_pool = query_rpc_pool_with_http_client(chain.rpc_urls, http);
    let quote =
        public_action_gas_fee_quote_from_rpc_pool(&query_rpc_pool, http.network_mode(), chain_id)
            .await
            .wrap_err("fetch public action gas price")?;
    let gas = resolve_self_broadcast_gas_fee(gas_fee, quote)?;
    Ok(public_native_action_gas_reserve_with_buffer(
        gas.max_fee_per_gas,
        steps,
        chain.gas.gas_limit_buffer,
    ))
}

pub(super) async fn public_action_gas_fee_quote_from_rpc_pool(
    query_rpc_pool: &QueryRpcPool,
    network_mode: crate::WalletNetworkMode,
    chain_id: u64,
) -> Result<PublicActionGasFeeQuote> {
    self_broadcast_gas_fee_quote_from_rpc_pool_with_tip_fallback(
        query_rpc_pool,
        network_mode,
        public_action_tip_fallback(chain_id),
    )
    .await
}

pub(super) const fn public_action_tip_fallback(chain_id: u64) -> SelfBroadcastTipFallback {
    if chain_id == PUBLIC_ACTION_BNB_CHAIN_ID {
        SelfBroadcastTipFallback::RpcGasPrice
    } else {
        SelfBroadcastTipFallback::Minimum
    }
}

const fn public_native_step_gas_units(step: PublicActionProgressStep) -> u64 {
    match step {
        PublicActionProgressStep::ShieldKey => 0,
        PublicActionProgressStep::Send => PUBLIC_NATIVE_SEND_GAS_UNITS,
        PublicActionProgressStep::Wrap => PUBLIC_NATIVE_WRAP_GAS_UNITS,
        PublicActionProgressStep::Approve => PUBLIC_NATIVE_APPROVE_GAS_UNITS,
        PublicActionProgressStep::Shield => PUBLIC_NATIVE_SHIELD_GAS_UNITS,
    }
}
