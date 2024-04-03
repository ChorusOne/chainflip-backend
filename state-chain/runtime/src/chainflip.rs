//! Configuration, utilities and helpers for the Chainflip runtime.
pub mod address_derivation;
pub mod backup_node_rewards;
pub mod cons_key_rotator;
pub mod decompose_recompose;
pub mod epoch_transition;
pub mod evm_vault_activator;
mod missed_authorship_slots;
mod offences;
mod signer_nomination;

use crate::{
	impl_transaction_builder_for_evm_chain, AccountId, AccountRoles, ArbitrumChainTracking,
	ArbitrumIngressEgress, Authorship, BitcoinChainTracking, BitcoinIngressEgress,
	BitcoinThresholdSigner, BlockNumber, Emissions, Environment, EthereumBroadcaster,
	EthereumChainTracking, EthereumIngressEgress, Flip, FlipBalance, Hash, PolkadotBroadcaster,
	PolkadotChainTracking, PolkadotIngressEgress, PolkadotThresholdSigner, Runtime, RuntimeCall,
	System, Validator, YEAR,
};
use backup_node_rewards::calculate_backup_rewards;
use cf_chains::{
	address::{
		to_encoded_address, try_from_encoded_address, AddressConverter, EncodedAddress,
		ForeignChainAddress,
	},
	arb::api::ArbitrumApi,
	assets::any::ForeignChainAndAsset,
	btc::{
		api::{BitcoinApi, SelectedUtxosAndChangeAmount, UtxoSelectionType},
		Bitcoin, BitcoinCrypto, BitcoinFeeInfo, BitcoinTransactionData, UtxoId,
	},
	dot::{
		api::PolkadotApi, Polkadot, PolkadotAccountId, PolkadotCrypto, PolkadotReplayProtection,
		PolkadotTransactionData, ResetProxyAccountNonce, RuntimeVersion,
	},
	eth::{
		self,
		api::{EthereumApi, StateChainGatewayAddressProvider},
		deposit_address::ETHEREUM_ETH_ADDRESS,
		Ethereum,
	},
	evm::{
		api::{EvmChainId, EvmEnvironmentProvider, EvmReplayProtection},
		EvmCrypto, Transaction,
	},
	AnyChain, ApiCall, Arbitrum, CcmChannelMetadata, CcmDepositMetadata, Chain, ChainCrypto,
	ChainEnvironment, ChainState, DepositChannel, ForeignChain, ReplayProtectionProvider,
	SetCommKeyWithAggKey, SetGovKeyWithAggKey, TransactionBuilder,
};
use cf_primitives::{chains::assets, AccountRole, Asset, BasisPoints, ChannelId};
use cf_traits::{
	AccountInfo, AccountRoleRegistry, BackupRewardsNotifier, BlockEmissions,
	BroadcastAnyChainGovKey, Broadcaster, Chainflip, CommKeyBroadcaster, DepositApi, EgressApi,
	EpochInfo, Heartbeat, IngressEgressFeeApi, Issuance, KeyProvider, OnBroadcastReady, OnDeposit,
	QualifyNode, RewardsDistribution, RuntimeUpgrade, ScheduledEgressDetails,
};
use codec::{Decode, Encode};
use eth::Address as EvmAddress;
use frame_support::{
	dispatch::{DispatchErrorWithPostInfo, PostDispatchInfo},
	pallet_prelude::DispatchError,
	sp_runtime::{
		traits::{BlockNumberProvider, One, UniqueSaturatedFrom, UniqueSaturatedInto},
		FixedPointNumber, FixedU64,
	},
	traits::{Defensive, Get},
};
pub use missed_authorship_slots::MissedAuraSlots;
pub use offences::*;
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
pub use signer_nomination::RandomSignerNomination;
use sp_core::U256;
use sp_std::prelude::*;

impl Chainflip for Runtime {
	type RuntimeCall = RuntimeCall;
	type Amount = FlipBalance;
	type ValidatorId = <Self as frame_system::Config>::AccountId;
	type EnsureWitnessed = pallet_cf_witnesser::EnsureWitnessed;
	type EnsurePrewitnessed = pallet_cf_witnesser::EnsurePrewitnessed;
	type EnsureWitnessedAtCurrentEpoch = pallet_cf_witnesser::EnsureWitnessedAtCurrentEpoch;
	type EnsureGovernance = pallet_cf_governance::EnsureGovernance;
	type EpochInfo = Validator;
	type AccountRoleRegistry = AccountRoles;
	type FundingInfo = Flip;
}

struct BackupNodeEmissions;

impl RewardsDistribution for BackupNodeEmissions {
	type Balance = FlipBalance;
	type Issuance = pallet_cf_flip::FlipIssuance<Runtime>;

	fn distribute() {
		let backup_nodes =
			Validator::highest_funded_qualified_backup_node_bids().collect::<Vec<_>>();
		if backup_nodes.is_empty() {
			return
		}

		// Distribute rewards one by one
		// N.B. This could be more optimal
		for (validator_id, reward) in calculate_backup_rewards(
			backup_nodes,
			Validator::bond(),
			<<Runtime as pallet_cf_reputation::Config>::HeartbeatBlockInterval as Get<
				BlockNumber,
			>>::get()
			.unique_saturated_into(),
			Emissions::backup_node_emission_per_block(),
			Emissions::current_authority_emission_per_block(),
			Self::Balance::unique_saturated_from(Validator::current_authority_count()),
		) {
			Flip::settle(&validator_id, Self::Issuance::mint(reward).into());
			<Emissions as BackupRewardsNotifier>::emit_event(&validator_id, reward);
		}
	}
}

pub struct ChainflipHeartbeat;

impl Heartbeat for ChainflipHeartbeat {
	type ValidatorId = AccountId;
	type BlockNumber = BlockNumber;

	fn on_heartbeat_interval() {
		<Emissions as BlockEmissions>::calculate_block_emissions();
		BackupNodeEmissions::distribute();
	}
}

/// Checks if the caller can execute free transactions
pub struct WaivedFees;

impl cf_traits::WaivedFees for WaivedFees {
	type AccountId = AccountId;
	type RuntimeCall = RuntimeCall;

	fn should_waive_fees(call: &Self::RuntimeCall, caller: &Self::AccountId) -> bool {
		if matches!(call, RuntimeCall::Governance(_)) {
			return super::Governance::members().contains(caller)
		}
		false
	}
}

/// We are willing to pay at most 2x the base fee. This is approximately the theoretical
/// limit of the rate of increase of the base fee over 6 blocks (12.5% per block).
const ETHEREUM_BASE_FEE_MULTIPLIER: FixedU64 = FixedU64::from_rational(2, 1);
/// Arbitrum has smaller variability so we are willing to pay at most 1.5x the base fee.
const ARBITRUM_BASE_FEE_MULTIPLIER: FixedU64 = FixedU64::from_rational(3, 2);
// We arbitrarily set the MAX_GAS_LIMIT
const ETHEREUM_MAX_GAS_LIMIT: u128 = 10_000_000;
const ARBITRUM_MAX_GAS_LIMIT: u128 = 25_000_000;

pub trait EvmPriorityFee<C: Chain> {
	fn get_priority_fee(_tracked_data: &C::TrackedData) -> Option<U256> {
		None
	}
}

impl EvmPriorityFee<Ethereum> for EthTransactionBuilder {
	fn get_priority_fee(tracked_data: &<Ethereum as Chain>::TrackedData) -> Option<U256> {
		Some(U256::from(tracked_data.priority_fee))
	}
}

impl EvmPriorityFee<Arbitrum> for ArbTransactionBuilder {
	// Setting the priority fee to zero to prevent the CFE from setting a different value
	fn get_priority_fee(_tracked_data: &<Arbitrum as Chain>::TrackedData) -> Option<U256> {
		Some(U256::from(0))
	}
}

pub struct EthTransactionBuilder;
pub struct ArbTransactionBuilder;
impl_transaction_builder_for_evm_chain!(
	Ethereum,
	EthTransactionBuilder,
	EthereumApi<EvmEnvironment>,
	EthereumChainTracking,
	ETHEREUM_BASE_FEE_MULTIPLIER,
	ETHEREUM_MAX_GAS_LIMIT
);
impl_transaction_builder_for_evm_chain!(
	Arbitrum,
	ArbTransactionBuilder,
	ArbitrumApi<EvmEnvironment>,
	ArbitrumChainTracking,
	ARBITRUM_BASE_FEE_MULTIPLIER,
	ARBITRUM_MAX_GAS_LIMIT
);

#[macro_export]
macro_rules! impl_transaction_builder_for_evm_chain {
	( $chain: ident, $transaction_builder: ident, $chain_api: ident <$env: ident>, $chain_tracking: ident, $base_fee_multiplier: expr, $max_gas_limit: expr ) => {
		impl TransactionBuilder<$chain, $chain_api<$env>> for $transaction_builder {
			fn build_transaction(
				signed_call: &$chain_api<$env>,
			) -> Transaction {
				Transaction {
					chain_id: signed_call.replay_protection().chain_id,
					contract: signed_call.replay_protection().contract_address,
					data: signed_call.chain_encoded(),
					gas_limit: Self::calculate_gas_limit(signed_call),
					..Default::default()
				}
			}

			fn refresh_unsigned_data(unsigned_tx: &mut Transaction) {
				if let Some(ChainState { tracked_data, .. }) = $chain_tracking::chain_state() {
					let max_fee_per_gas = tracked_data.max_fee_per_gas($base_fee_multiplier);
					unsigned_tx.max_fee_per_gas = Some(U256::from(max_fee_per_gas));
					unsigned_tx.max_priority_fee_per_gas = $transaction_builder::get_priority_fee(&tracked_data);
				} else {
					log::warn!("No chain data for {}. This should never happen. Please check Chain Tracking data.", $chain::NAME);
				}
			}

			fn requires_signature_refresh(
				_call: &$chain_api<$env>,
				_payload: &<EvmCrypto as ChainCrypto>::Payload,
			) -> bool {
				false
			}

			/// Calculate the gas limit for a this evm chain's call, using the current gas price.
			/// Currently for only CCM calls, the gas limit is calculated as:
			/// Gas limit = gas_budget / (multiplier * base_gas_price + priority_fee)
			/// All other calls uses a default gas limit. Multiplier=1 to avoid user overpaying for gas.
			/// The max_fee_per_gas will still have the default this evm chain's base fee multiplier applied.
			fn calculate_gas_limit(call: &$chain_api<$env>) -> Option<U256> {
				if let Some(gas_budget) = call.gas_budget() {
					let current_fee_per_gas = $chain_tracking::chain_state()
						.or_else(||{
							log::warn!("No chain data for {}. This should never happen. Please check Chain Tracking data.", $chain::NAME);
							None
						})?
						.tracked_data
						.max_fee_per_gas(One::one());
					Some(gas_budget
						.checked_div(current_fee_per_gas)
						.unwrap_or_else(||{
							log::warn!("Current gas price for {} is 0. This should never happen. Please check Chain Tracking data.", $chain::NAME);
							Default::default()
						}).min($max_gas_limit)
						.into())
				} else {
					None
				}
			}
		}
	}
}

pub struct DotTransactionBuilder;
impl TransactionBuilder<Polkadot, PolkadotApi<DotEnvironment>> for DotTransactionBuilder {
	fn build_transaction(
		signed_call: &PolkadotApi<DotEnvironment>,
	) -> <Polkadot as Chain>::Transaction {
		PolkadotTransactionData { encoded_extrinsic: signed_call.chain_encoded() }
	}

	fn refresh_unsigned_data(_unsigned_tx: &mut <Polkadot as Chain>::Transaction) {
		// TODO: For now this is a noop until we actually have dot chain tracking
	}

	fn requires_signature_refresh(
		call: &PolkadotApi<DotEnvironment>,
		payload: &<<Polkadot as Chain>::ChainCrypto as ChainCrypto>::Payload,
	) -> bool {
		// Current key and signature are irrelevant. The only thing that can invalidate a polkadot
		// transaction is if the payload changes due to a runtime version update.
		&call.threshold_signature_payload() != payload
	}
}

pub struct BtcTransactionBuilder;
impl TransactionBuilder<Bitcoin, BitcoinApi<BtcEnvironment>> for BtcTransactionBuilder {
	fn build_transaction(
		signed_call: &BitcoinApi<BtcEnvironment>,
	) -> <Bitcoin as Chain>::Transaction {
		BitcoinTransactionData { encoded_transaction: signed_call.chain_encoded() }
	}

	fn refresh_unsigned_data(_unsigned_tx: &mut <Bitcoin as Chain>::Transaction) {
		// Since BTC txs are chained and the subsequent tx depends on the success of the previous
		// one, changing the BTC tx fee will mean all subsequent txs are also invalid and so
		// refreshing btc tx is not trivial. We leave it a no-op for now.
	}

	fn requires_signature_refresh(
		_call: &BitcoinApi<BtcEnvironment>,
		_payload: &<<Bitcoin as Chain>::ChainCrypto as ChainCrypto>::Payload,
	) -> bool {
		// The payload for a Bitcoin transaction will never change and so it doesnt need to be
		// checked here. We also dont need to check for the signature here because even if we are in
		// the next epoch and the key has changed, the old signature for the btc tx is still valid
		// since its based on those old input UTXOs. In fact, we never have to resign btc txs and
		// the btc tx is always valid as long as the input UTXOs are valid. Therefore, we don't have
		// to check anything here and just rebroadcast.
		false
	}
}

pub struct BlockAuthorRewardDistribution;

impl RewardsDistribution for BlockAuthorRewardDistribution {
	type Balance = FlipBalance;
	type Issuance = pallet_cf_flip::FlipIssuance<Runtime>;

	fn distribute() {
		let reward_amount = Emissions::current_authority_emission_per_block();
		if reward_amount != 0 {
			if let Some(current_block_author) = Authorship::author() {
				Flip::settle(&current_block_author, Self::Issuance::mint(reward_amount).into());
			} else {
				log::warn!("No block author for block {}.", System::current_block_number());
			}
		}
	}
}
pub struct RuntimeUpgradeManager;

impl RuntimeUpgrade for RuntimeUpgradeManager {
	fn do_upgrade(code: Vec<u8>) -> Result<PostDispatchInfo, DispatchErrorWithPostInfo> {
		System::set_code(frame_system::RawOrigin::Root.into(), code)
	}
}
pub struct EvmEnvironment;

impl<C: Chain<ReplayProtection = EvmReplayProtection, ReplayProtectionParams = EvmAddress>>
	ReplayProtectionProvider<C> for EvmEnvironment
where
	EvmEnvironment: EvmEnvironmentProvider<C>,
{
	fn replay_protection(
		contract_address: <C as Chain>::ReplayProtectionParams,
	) -> EvmReplayProtection {
		EvmReplayProtection {
			nonce: <Self as EvmEnvironmentProvider<C>>::next_nonce(),
			chain_id: <Self as EvmEnvironmentProvider<C>>::chain_id(),
			key_manager_address: <Self as EvmEnvironmentProvider<C>>::key_manager_address(),
			contract_address,
		}
	}
}

impl EvmEnvironmentProvider<Ethereum> for EvmEnvironment {
	fn token_address(asset: assets::eth::Asset) -> Option<EvmAddress> {
		match asset {
			assets::eth::Asset::Eth => Some(ETHEREUM_ETH_ADDRESS),
			erc20 => Environment::supported_eth_assets(erc20).map(Into::into),
		}
	}

	fn vault_address() -> EvmAddress {
		Environment::eth_vault_address()
	}

	fn key_manager_address() -> EvmAddress {
		Environment::key_manager_address()
	}

	fn chain_id() -> cf_chains::evm::api::EvmChainId {
		Environment::ethereum_chain_id()
	}

	fn next_nonce() -> u64 {
		Environment::next_ethereum_signature_nonce()
	}
}

// state chain gateway address only exists for Ethereum and does not exist for any other Evm Chain
impl StateChainGatewayAddressProvider for EvmEnvironment {
	fn state_chain_gateway_address() -> EvmAddress {
		Environment::state_chain_gateway_address()
	}
}

impl EvmEnvironmentProvider<Arbitrum> for EvmEnvironment {
	fn token_address(asset: assets::arb::Asset) -> Option<EvmAddress> {
		match asset {
			assets::arb::Asset::ArbEth => Some(ETHEREUM_ETH_ADDRESS),
			assets::arb::Asset::ArbUsdc => Environment::supported_arb_assets(asset).map(Into::into),
		}
	}

	fn vault_address() -> EvmAddress {
		Environment::arb_vault_address()
	}

	fn key_manager_address() -> EvmAddress {
		Environment::arb_key_manager_address()
	}

	fn chain_id() -> EvmChainId {
		Environment::arbitrum_chain_id()
	}

	fn next_nonce() -> u64 {
		Environment::next_arbitrum_signature_nonce()
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct DotEnvironment;

impl ReplayProtectionProvider<Polkadot> for DotEnvironment {
	// Get the Environment values for vault_account, NetworkChoice and the next nonce for the
	// proxy_account
	fn replay_protection(reset_nonce: ResetProxyAccountNonce) -> PolkadotReplayProtection {
		PolkadotReplayProtection {
			genesis_hash: Environment::polkadot_genesis_hash(),
			// It should not be possible to get None here, since we never send
			// any transactions unless we have a vault account and associated
			// proxy.
			signer: <PolkadotThresholdSigner as KeyProvider<PolkadotCrypto>>::active_epoch_key()
				.map(|epoch_key| epoch_key.key)
				.defensive_unwrap_or_default(),
			nonce: Environment::next_polkadot_proxy_account_nonce(reset_nonce),
		}
	}
}

impl Get<RuntimeVersion> for DotEnvironment {
	fn get() -> RuntimeVersion {
		PolkadotChainTracking::chain_state().unwrap().tracked_data.runtime_version
	}
}

impl ChainEnvironment<cf_chains::dot::api::VaultAccount, PolkadotAccountId> for DotEnvironment {
	fn lookup(_: cf_chains::dot::api::VaultAccount) -> Option<PolkadotAccountId> {
		Environment::polkadot_vault_account()
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct BtcEnvironment;

impl ReplayProtectionProvider<Bitcoin> for BtcEnvironment {
	fn replay_protection(_params: ()) {}
}

impl ChainEnvironment<UtxoSelectionType, SelectedUtxosAndChangeAmount> for BtcEnvironment {
	fn lookup(utxo_selection_type: UtxoSelectionType) -> Option<SelectedUtxosAndChangeAmount> {
		Environment::select_and_take_bitcoin_utxos(utxo_selection_type)
	}
}

impl ChainEnvironment<(), cf_chains::btc::AggKey> for BtcEnvironment {
	fn lookup(_: ()) -> Option<cf_chains::btc::AggKey> {
		<BitcoinThresholdSigner as KeyProvider<BitcoinCrypto>>::active_epoch_key()
			.map(|epoch_key| epoch_key.key)
	}
}

pub struct TokenholderGovernanceBroadcaster;

impl TokenholderGovernanceBroadcaster {
	fn broadcast_gov_key<C, B>(maybe_old_key: Option<Vec<u8>>, new_key: Vec<u8>) -> Result<(), ()>
	where
		C: Chain,
		B: Broadcaster<C>,
		<B as Broadcaster<C>>::ApiCall: cf_chains::SetGovKeyWithAggKey<C::ChainCrypto>,
	{
		let maybe_old_key = if let Some(old_key) = maybe_old_key {
			Some(Decode::decode(&mut &old_key[..]).or(Err(()))?)
		} else {
			None
		};
		let api_call = SetGovKeyWithAggKey::<C::ChainCrypto>::new_unsigned(
			maybe_old_key,
			Decode::decode(&mut &new_key[..]).or(Err(()))?,
		)?;
		B::threshold_sign_and_broadcast(api_call);
		Ok(())
	}

	fn is_govkey_compatible<C: ChainCrypto>(key: &[u8]) -> bool {
		C::GovKey::decode(&mut &key[..]).is_ok()
	}
}

impl BroadcastAnyChainGovKey for TokenholderGovernanceBroadcaster {
	fn broadcast_gov_key(
		chain: ForeignChain,
		maybe_old_key: Option<Vec<u8>>,
		new_key: Vec<u8>,
	) -> Result<(), ()> {
		match chain {
			ForeignChain::Ethereum =>
				Self::broadcast_gov_key::<Ethereum, EthereumBroadcaster>(maybe_old_key, new_key),
			ForeignChain::Polkadot =>
				Self::broadcast_gov_key::<Polkadot, PolkadotBroadcaster>(maybe_old_key, new_key),
			ForeignChain::Bitcoin => Err(()),
			ForeignChain::Arbitrum => Err(()),
		}
	}

	fn is_govkey_compatible(chain: ForeignChain, key: &[u8]) -> bool {
		match chain {
			ForeignChain::Ethereum =>
				Self::is_govkey_compatible::<<Ethereum as Chain>::ChainCrypto>(key),
			ForeignChain::Polkadot =>
				Self::is_govkey_compatible::<<Polkadot as Chain>::ChainCrypto>(key),
			ForeignChain::Bitcoin => false,
			ForeignChain::Arbitrum => false,
		}
	}
}

impl CommKeyBroadcaster for TokenholderGovernanceBroadcaster {
	fn broadcast(new_key: <<Ethereum as Chain>::ChainCrypto as ChainCrypto>::GovKey) {
		<EthereumBroadcaster as Broadcaster<Ethereum>>::threshold_sign_and_broadcast(
			SetCommKeyWithAggKey::<EvmCrypto>::new_unsigned(new_key),
		);
	}
}

#[macro_export]
macro_rules! impl_deposit_api_for_anychain {
	( $t: ident, $(($chain: ident, $pallet: ident)),+ ) => {
		impl DepositApi<AnyChain> for $t {
			type AccountId = <Runtime as frame_system::Config>::AccountId;
			type Amount = <Runtime as Chainflip>::Amount;

			fn request_liquidity_deposit_address(
				lp_account: Self::AccountId,
				source_asset: Asset,
				boost_fee: BasisPoints
			) -> Result<(ChannelId, ForeignChainAddress, <AnyChain as cf_chains::Chain>::ChainBlockNumber, FlipBalance), DispatchError> {
				match source_asset.into() {
					$(
						ForeignChainAndAsset::$chain(source_asset) =>
							$pallet::request_liquidity_deposit_address(
								lp_account,
								source_asset,
								boost_fee
							).map(|(channel, address, block_number, channel_opening_fee)| (channel, address, block_number.into(), channel_opening_fee)),
					)+
				}
			}

			fn request_swap_deposit_address(
				source_asset: Asset,
				destination_asset: Asset,
				destination_address: ForeignChainAddress,
				broker_commission_bps: BasisPoints,
				broker_id: Self::AccountId,
				channel_metadata: Option<CcmChannelMetadata>,
				boost_fee: BasisPoints
			) -> Result<(ChannelId, ForeignChainAddress, <AnyChain as cf_chains::Chain>::ChainBlockNumber, FlipBalance), DispatchError> {
				match source_asset.into() {
					$(
						ForeignChainAndAsset::$chain(source_asset) => $pallet::request_swap_deposit_address(
							source_asset,
							destination_asset,
							destination_address,
							broker_commission_bps,
							broker_id,
							channel_metadata,
							boost_fee
						).map(|(channel, address, block_number, channel_opening_fee)| (channel, address, block_number.into(), channel_opening_fee)),
					)+
				}
			}
		}
	}
}

#[macro_export]
macro_rules! impl_egress_api_for_anychain {
	( $t: ident, $(($chain: ident, $pallet: ident)),+ ) => {
		impl EgressApi<AnyChain> for $t {
			type EgressError = DispatchError;

			fn schedule_egress(
				asset: Asset,
				amount: <AnyChain as Chain>::ChainAmount,
				destination_address: <AnyChain as Chain>::ChainAccount,
				maybe_ccm_with_gas_budget: Option<(CcmDepositMetadata, <AnyChain as Chain>::ChainAmount)>,
			) -> Result<ScheduledEgressDetails<AnyChain>, DispatchError> {
				match asset.into() {
					$(
						ForeignChainAndAsset::$chain(asset) => $pallet::schedule_egress(
							asset,
							amount.try_into().expect("Checked for amount compatibility"),
							destination_address
								.try_into()
								.expect("This address cast is ensured to succeed."),
								maybe_ccm_with_gas_budget.map(|(metadata, gas_budget)| (metadata, gas_budget.try_into().expect("Chain's Amount must be compatible with u128."))),
						)
						.map(|ScheduledEgressDetails { egress_id, egress_amount, fee_withheld }| ScheduledEgressDetails { egress_id, egress_amount: egress_amount.into(), fee_withheld: fee_withheld.into() })
						.map_err(Into::into),
					)+
				}
			}
		}
	}
}

pub struct AnyChainIngressEgressHandler;
impl_deposit_api_for_anychain!(
	AnyChainIngressEgressHandler,
	(Ethereum, EthereumIngressEgress),
	(Polkadot, PolkadotIngressEgress),
	(Bitcoin, BitcoinIngressEgress),
	(Arbitrum, ArbitrumIngressEgress)
);

impl_egress_api_for_anychain!(
	AnyChainIngressEgressHandler,
	(Ethereum, EthereumIngressEgress),
	(Polkadot, PolkadotIngressEgress),
	(Bitcoin, BitcoinIngressEgress),
	(Arbitrum, ArbitrumIngressEgress)
);

pub struct DepositHandler;
impl OnDeposit<Ethereum> for DepositHandler {}
impl OnDeposit<Polkadot> for DepositHandler {}
impl OnDeposit<Bitcoin> for DepositHandler {
	fn on_deposit_made(
		utxo_id: <Bitcoin as Chain>::DepositDetails,
		amount: <Bitcoin as Chain>::ChainAmount,
		channel: DepositChannel<Bitcoin>,
	) {
		Environment::add_bitcoin_utxo_to_list(amount, utxo_id, channel.state)
	}
}
impl OnDeposit<Arbitrum> for DepositHandler {}

pub struct ChainAddressConverter;

impl AddressConverter for ChainAddressConverter {
	fn to_encoded_address(address: ForeignChainAddress) -> EncodedAddress {
		to_encoded_address(address, Environment::network_environment)
	}

	fn try_from_encoded_address(
		encoded_address: EncodedAddress,
	) -> Result<ForeignChainAddress, ()> {
		try_from_encoded_address(encoded_address, Environment::network_environment)
	}
}

pub struct BroadcastReadyProvider;
impl OnBroadcastReady<Ethereum> for BroadcastReadyProvider {
	type ApiCall = EthereumApi<EvmEnvironment>;
}
impl OnBroadcastReady<Polkadot> for BroadcastReadyProvider {
	type ApiCall = PolkadotApi<DotEnvironment>;
}
impl OnBroadcastReady<Bitcoin> for BroadcastReadyProvider {
	type ApiCall = BitcoinApi<BtcEnvironment>;

	fn on_broadcast_ready(api_call: &Self::ApiCall) {
		match api_call {
			BitcoinApi::BatchTransfer(batch_transfer) => {
				let tx_id = batch_transfer.bitcoin_transaction.txid();
				let outputs = batch_transfer.bitcoin_transaction.outputs.clone();
				let output_len = outputs.len();
				let vout = output_len - 1;
				let change_output = outputs.get(vout).unwrap();
				Environment::add_bitcoin_change_utxo(
					change_output.amount,
					UtxoId { tx_id, vout: vout as u32 },
					batch_transfer.change_utxo_key,
				);
			},
			_ => unreachable!(),
		}
	}
}
impl OnBroadcastReady<Arbitrum> for BroadcastReadyProvider {
	type ApiCall = ArbitrumApi<EvmEnvironment>;
}

pub struct BitcoinFeeGetter;
impl cf_traits::GetBitcoinFeeInfo for BitcoinFeeGetter {
	fn bitcoin_fee_info() -> BitcoinFeeInfo {
		BitcoinChainTracking::chain_state().unwrap().tracked_data.btc_fee_info
	}
}

pub struct ValidatorRoleQualification;

impl QualifyNode<<Runtime as Chainflip>::ValidatorId> for ValidatorRoleQualification {
	fn is_qualified(id: &<Runtime as Chainflip>::ValidatorId) -> bool {
		AccountRoles::has_account_role(id, AccountRole::Validator)
	}
}

// Calculates the APY of a given account, returned in Basis Points (1 b.p. = 0.01%)
// Returns Some(APY) if the account is a Validator/backup validator.
// Otherwise returns None.
pub fn calculate_account_apy(account_id: &AccountId) -> Option<u32> {
	if pallet_cf_validator::CurrentAuthorities::<Runtime>::get().contains(account_id) {
		// Authority: reward is earned by authoring a block.
		Some(
			Emissions::current_authority_emission_per_block() * YEAR as u128 /
				pallet_cf_validator::CurrentAuthorities::<Runtime>::decode_non_dedup_len()
					.expect("Current authorities must exists and non-empty.") as u128,
		)
	} else {
		let backups_earning_rewards =
			Validator::highest_funded_qualified_backup_node_bids().collect::<Vec<_>>();
		if backups_earning_rewards.iter().any(|bid| bid.bidder_id == *account_id) {
			// Calculate backup validator reward for the current block, then scaled linearly into
			// YEAR.
			calculate_backup_rewards::<AccountId, FlipBalance>(
				backups_earning_rewards,
				Validator::bond(),
				One::one(),
				Emissions::backup_node_emission_per_block(),
				Emissions::current_authority_emission_per_block(),
				u128::from(Validator::current_authority_count()),
			)
			.into_iter()
			.find(|(id, _reward)| *id == *account_id)
			.map(|(_id, reward)| reward * YEAR as u128)
		} else {
			None
		}
	}
	.map(|reward_pa| {
		// Convert Permill to Basis Point.
		FixedU64::from_rational(reward_pa, Flip::balance(account_id))
			.checked_mul_int(10_000u32)
			.unwrap_or_default()
	})
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Encode, Decode)]
pub struct BlockUpdate<Data> {
	pub block_hash: Hash,
	pub block_number: BlockNumber,
	// NOTE: Flatten requires Data types to be struct or map
	// Also flatten is incompatible with u128, so AssetAmounts needs to be String type.
	#[serde(flatten)]
	pub data: Data,
}

#[macro_export]
macro_rules! impl_ingress_egress_fee_api_for_anychain {
	( $t: ident, $(($chain: ident, $pallet: ident)),+ ) => {
		impl IngressEgressFeeApi<AnyChain> for $t {
			fn accrue_withheld_fee(asset: Asset, fee: <AnyChain as Chain>::ChainAmount) {
				match asset.into() {
					$(
						ForeignChainAndAsset::$chain(asset) => $pallet::accrue_withheld_fee(
							asset,
							fee.try_into().expect("Checked for amount compatibility"),
						),
					)+
				}
			}
		}
	}
}

pub struct IngressEgressFeeHandler;
impl_ingress_egress_fee_api_for_anychain!(
	IngressEgressFeeHandler,
	(Ethereum, EthereumIngressEgress),
	(Polkadot, PolkadotIngressEgress),
	(Bitcoin, BitcoinIngressEgress),
	(Arbitrum, ArbitrumIngressEgress)
);
