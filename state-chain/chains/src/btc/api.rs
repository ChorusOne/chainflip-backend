pub mod batch_transfer;

use super::{
	deposit_address::derive_btc_deposit_bitcoin_script, AggKey, Bitcoin, BitcoinOutput, BtcAmount,
	Utxo, CHANGE_ADDRESS_SALT,
};
use crate::*;
use frame_support::{CloneNoBound, DebugNoBound, EqNoBound, Never, PartialEqNoBound};
use sp_std::marker::PhantomData;

#[derive(CloneNoBound, DebugNoBound, PartialEqNoBound, EqNoBound, Encode, Decode, TypeInfo)]
#[scale_info(skip_type_params(Environment))]
pub enum BitcoinApi<Environment: 'static> {
	BatchTransfer(batch_transfer::BatchTransfer),
	#[doc(hidden)]
	#[codec(skip)]
	_Phantom(PhantomData<Environment>, Never),
}

pub type SelectedUtxosAndChangeAmount = (Vec<Utxo>, BtcAmount);

#[derive(Copy, Clone, RuntimeDebug, PartialEq, Eq, Encode, Decode, MaxEncodedLen, TypeInfo)]
pub enum UtxoSelectionType {
	SelectAllForRotation,
	Some { output_amount: BtcAmount, number_of_outputs: u64 },
}

impl<E> AllBatch<Bitcoin> for BitcoinApi<E>
where
	E: ChainEnvironment<UtxoSelectionType, SelectedUtxosAndChangeAmount>
		+ ChainEnvironment<(), AggKey>,
{
	fn new_unsigned(
		_fetch_params: Vec<FetchAssetParams<Bitcoin>>,
		transfer_params: Vec<TransferAssetParams<Bitcoin>>,
	) -> Result<Self, ()> {
		let agg_key @ AggKey { current, .. } =
			<E as ChainEnvironment<(), AggKey>>::lookup(()).ok_or(())?;
		let bitcoin_change_script = derive_btc_deposit_bitcoin_script(current, CHANGE_ADDRESS_SALT);
		let mut total_output_amount: u64 = 0;
		let mut btc_outputs = vec![];
		for transfer_param in transfer_params {
			btc_outputs.push(BitcoinOutput {
				amount: transfer_param.amount,
				script_pubkey: transfer_param.to.into(),
			});
			total_output_amount += transfer_param.amount;
		}
		// Looks up all available Utxos and selects and takes them for the transaction depending on
		// the amount that needs to be output.
		let (selected_input_utxos, change_amount) = E::lookup(UtxoSelectionType::Some {
			output_amount: total_output_amount,
			number_of_outputs: (btc_outputs.len() + 1) as u64, // +1 for the change output
		})
		.ok_or(())?;

		btc_outputs
			.push(BitcoinOutput { amount: change_amount, script_pubkey: bitcoin_change_script });

		Ok(Self::BatchTransfer(batch_transfer::BatchTransfer::new_unsigned(
			&agg_key,
			agg_key.current,
			selected_input_utxos,
			btc_outputs,
		)))
	}
}

impl<E> SetAggKeyWithAggKey<Bitcoin> for BitcoinApi<E>
where
	E: ChainEnvironment<UtxoSelectionType, SelectedUtxosAndChangeAmount>,
{
	fn new_unsigned(
		maybe_old_key: Option<<Bitcoin as ChainCrypto>::AggKey>,
		new_key: <Bitcoin as ChainCrypto>::AggKey,
	) -> Result<Self, ()> {
		// We will use the bitcoin address derived with the salt of 0 as the vault address where we
		// collect unspent amounts in btc transactions and consolidate funds when rotating epoch.
		let new_vault_change_script =
			derive_btc_deposit_bitcoin_script(new_key.current, CHANGE_ADDRESS_SALT);

		// Max possible btc value to get all available utxos
		// If we don't have any UTXOs then we're not required to do this.
		let (all_input_utxos, change_amount) =
			E::lookup(UtxoSelectionType::SelectAllForRotation).ok_or(())?;

		Ok(Self::BatchTransfer(batch_transfer::BatchTransfer::new_unsigned(
			&maybe_old_key.ok_or(())?,
			new_key.current,
			all_input_utxos,
			vec![BitcoinOutput { amount: change_amount, script_pubkey: new_vault_change_script }],
		)))
	}
}

impl<E> From<batch_transfer::BatchTransfer> for BitcoinApi<E> {
	fn from(tx: batch_transfer::BatchTransfer) -> Self {
		Self::BatchTransfer(tx)
	}
}

// TODO: Implement transfer / transfer and call for Bitcoin.
impl<E: ReplayProtectionProvider<Bitcoin>> ExecutexSwapAndCall<Bitcoin> for BitcoinApi<E> {
	fn new_unsigned(
		_egress_id: EgressId,
		_transfer_param: TransferAssetParams<Bitcoin>,
		_source_address: ForeignChainAddress,
		_message: Vec<u8>,
	) -> Result<Self, DispatchError> {
		Err(DispatchError::Other("Bitcoin's ExecutexSwapAndCall is not supported."))
	}
}

impl<E> ApiCall<Bitcoin> for BitcoinApi<E> {
	fn threshold_signature_payload(&self) -> <Bitcoin as ChainCrypto>::Payload {
		match self {
			BitcoinApi::BatchTransfer(tx) => tx.threshold_signature_payload(),

			BitcoinApi::_Phantom(..) => unreachable!(),
		}
	}

	fn signed(self, threshold_signature: &<Bitcoin as ChainCrypto>::ThresholdSignature) -> Self {
		match self {
			BitcoinApi::BatchTransfer(call) => call.signed(threshold_signature).into(),

			BitcoinApi::_Phantom(..) => unreachable!(),
		}
	}

	fn chain_encoded(&self) -> Vec<u8> {
		match self {
			BitcoinApi::BatchTransfer(call) => call.chain_encoded(),

			BitcoinApi::_Phantom(..) => unreachable!(),
		}
	}

	fn is_signed(&self) -> bool {
		match self {
			BitcoinApi::BatchTransfer(call) => call.is_signed(),

			BitcoinApi::_Phantom(..) => unreachable!(),
		}
	}

	fn transaction_out_id(&self) -> <Bitcoin as ChainCrypto>::TransactionOutId {
		match self {
			BitcoinApi::BatchTransfer(call) => call.transaction_out_id(),
			BitcoinApi::_Phantom(..) => unreachable!(),
		}
	}
}
