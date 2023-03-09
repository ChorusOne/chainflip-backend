use std::{collections::BTreeSet, sync::Arc};

use crate::constants::BTC_INGRESS_BLOCK_SAFETY_MARGIN;
use bitcoincore_rpc::bitcoin::{Transaction, TxOut};
use cf_chains::Bitcoin;
use futures::StreamExt;
use tokio::select;
use tracing::{info, info_span, trace, Instrument};

use crate::{
	multisig::{ChainTag, PersistentKeyDB},
	witnesser::{
		block_head_stream_from::block_head_stream_from,
		checkpointing::{
			get_witnesser_start_block_with_checkpointing, StartCheckpointing, WitnessedUntil,
		},
		epoch_witnesser::{self},
		http_safe_stream::{safe_polling_http_head_stream, HTTP_POLL_INTERVAL},
		EpochStart,
	},
};

use super::{
	rpc::{BtcRpcApi, BtcRpcClient},
	ScriptPubKey,
};

// Takes txs and list of monitored addresses. Returns a list of txs that are relevant to the
// monitored addresses.
pub fn filter_interesting_utxos(
	txs: Vec<Transaction>,
	monitored_script_pubkeys: &BTreeSet<ScriptPubKey>,
) -> Vec<TxOut> {
	let mut interesting_utxos = vec![];
	for tx in txs {
		for tx_out in &tx.output {
			if tx_out.value > 0 {
				let script_pubkey_bytes = tx_out.script_pubkey.to_bytes();
				if monitored_script_pubkeys.contains(&script_pubkey_bytes) {
					interesting_utxos.push(tx_out.clone());
				}
			}
		}
	}
	interesting_utxos
}

pub async fn start(
	epoch_starts_receiver: async_broadcast::Receiver<EpochStart<Bitcoin>>,
	btc_rpc: BtcRpcClient,
	script_pubkeys_receiver: tokio::sync::mpsc::UnboundedReceiver<ScriptPubKey>,
	monitored_script_pubkeys: BTreeSet<ScriptPubKey>,
	db: Arc<PersistentKeyDB>,
) -> Result<(), (async_broadcast::Receiver<EpochStart<Bitcoin>>, anyhow::Error)> {
	epoch_witnesser::start(
		epoch_starts_receiver,
		|_epoch_start| true,
		(script_pubkeys_receiver, monitored_script_pubkeys),
		move |mut end_witnessing_receiver,
		      epoch_start,
		      (mut script_pubkeys_receiver, mut monitored_script_pubkeys)| {
			let db = db.clone();
			let btc_rpc = btc_rpc.clone();
			async move {
				// TODO: Look at deduplicating this
				let (from_block, witnessed_until_sender) =
					match get_witnesser_start_block_with_checkpointing::<cf_chains::Bitcoin>(
						ChainTag::Bitcoin,
						epoch_start.epoch_index,
						epoch_start.block_number,
						db,
					)
					.await
					.expect("Failed to start Btc witnesser checkpointing")
					{
						StartCheckpointing::Started((from_block, witnessed_until_sender)) =>
							(from_block, witnessed_until_sender),
						StartCheckpointing::AlreadyWitnessedEpoch =>
							return Result::<_, anyhow::Error>::Ok((
								script_pubkeys_receiver,
								monitored_script_pubkeys,
							)),
					};

				let mut block_number_stream_from = block_head_stream_from(
					from_block,
					safe_polling_http_head_stream(
						btc_rpc.clone(),
						HTTP_POLL_INTERVAL,
						BTC_INGRESS_BLOCK_SAFETY_MARGIN,
					)
					.await,
					move |block_number| futures::future::ready(Ok(block_number)),
				)
				.await?;

				let mut end_at_block = None;
				let mut current_block = from_block;

				loop {
					let block_number = select! {
						end_block = &mut end_witnessing_receiver => {
							end_at_block = Some(end_block.expect("end witnessing channel was dropped unexpectedly"));
							None
						}
						Some(block_number) = block_number_stream_from.next()  => {
							current_block = block_number;
							Some(block_number)
						}
					};

					if let Some(end_block) = end_at_block {
						if current_block >= end_block {
							info!("Btc block witnessers unsubscribe at block {end_block}");
							break
						}
					}

					if let Some(block_number) = block_number {
						let block = btc_rpc.block(btc_rpc.block_hash(block_number)?)?;

						while let Ok(script_pubkey) = script_pubkeys_receiver.try_recv() {
							monitored_script_pubkeys.insert(script_pubkey);
						}

						trace!("Checking BTC block: {block_number} for interesting UTXOs");

						let interesting_utxos =
							filter_interesting_utxos(block.txdata, &monitored_script_pubkeys);

						for utxo in interesting_utxos {
							info!("Witnessing BTC ingress UTXO: {:?}", utxo);
							todo!("Witness BTC utxo to SC: {:?}", utxo);
						}

						witnessed_until_sender
							.send(WitnessedUntil {
								epoch_index: epoch_start.epoch_index,
								block_number,
							})
							.await
							.unwrap();
					}
				}

				Ok((script_pubkeys_receiver, monitored_script_pubkeys))
			}
		},
	)
	.instrument(info_span!("BTC-Witnesser"))
	.await
}

#[cfg(test)]
mod tests {
	use crate::settings;

	use super::*;

	#[ignore = "Requires a running BTC node"]
	#[tokio::test]
	async fn test_btc_witnesser() {
		crate::logging::init_json_logger();

		let rpc = BtcRpcClient::new(&settings::Btc {
			http_node_endpoint: "http://127.0.0.1:18443".to_string(),
			rpc_user: "kyle".to_string(),
			rpc_password: "password".to_string(),
		})
		.unwrap();

		let (_script_pubkeys_sender, script_pubkeys_receiver) =
			tokio::sync::mpsc::unbounded_channel();

		let (epoch_starts_sender, epoch_starts_receiver) = async_broadcast::broadcast(1);

		let (_dir, db_path) = crate::testing::new_temp_directory_with_nonexistent_file();
		let db = PersistentKeyDB::open_and_migrate_to_latest(&db_path, None).unwrap();

		epoch_starts_sender
			.broadcast(EpochStart {
				epoch_index: 1,
				block_number: 56,
				current: true,
				participant: true,
				data: (),
			})
			.await
			.unwrap();

		start(epoch_starts_receiver, rpc, script_pubkeys_receiver, BTreeSet::new(), Arc::new(db))
			.await
			.unwrap();
	}
}

#[cfg(test)]
mod test_utxo_filtering {
	use bitcoincore_rpc::bitcoin::{PackedLockTime, Script, Transaction, TxOut};

	use super::*;

	fn fake_transaction(tx_outs: Vec<TxOut>) -> Transaction {
		Transaction { version: 2, lock_time: PackedLockTime(0), input: vec![], output: tx_outs }
	}

	#[test]
	fn filter_interesting_utxos_no_utxos() {
		let txs = vec![fake_transaction(vec![]), fake_transaction(vec![])];
		let monitored_script_pubkeys = BTreeSet::new();
		assert!(filter_interesting_utxos(txs, &monitored_script_pubkeys).is_empty());
	}

	#[test]
	fn filter_interesting_utxos_several_same_tx() {
		let monitored_pubkey = vec![0, 1, 2, 3];
		let txs = vec![
			fake_transaction(vec![
				TxOut { value: 2324, script_pubkey: Script::from(monitored_pubkey.clone()) },
				TxOut { value: 12223, script_pubkey: Script::from(vec![0, 32, 121, 9]) },
				TxOut { value: 1234, script_pubkey: Script::from(monitored_pubkey.clone()) },
			]),
			fake_transaction(vec![]),
		];
		let monitored_script_pubkeys = BTreeSet::from([monitored_pubkey]);
		let interesting_utxos = filter_interesting_utxos(txs, &monitored_script_pubkeys);
		assert_eq!(interesting_utxos.len(), 2);
		assert_eq!(interesting_utxos[0].value, 2324);
		assert_eq!(interesting_utxos[1].value, 1234);
	}

	#[test]
	fn filter_interesting_utxos_several_diff_tx() {
		let monitored_pubkey = vec![0, 1, 2, 3];
		let txs = vec![
			fake_transaction(vec![
				TxOut { value: 2324, script_pubkey: Script::from(monitored_pubkey.clone()) },
				TxOut { value: 12223, script_pubkey: Script::from(vec![0, 32, 121, 9]) },
			]),
			fake_transaction(vec![TxOut {
				value: 1234,
				script_pubkey: Script::from(monitored_pubkey.clone()),
			}]),
		];
		let monitored_script_pubkeys = BTreeSet::from([monitored_pubkey]);
		let interesting_utxos = filter_interesting_utxos(txs, &monitored_script_pubkeys);
		assert_eq!(interesting_utxos.len(), 2);
		assert_eq!(interesting_utxos[0].value, 2324);
		assert_eq!(interesting_utxos[1].value, 1234);
	}

	#[test]
	fn filter_out_value_0() {
		let monitored_pubkey = vec![0, 1, 2, 3];
		let txs = vec![fake_transaction(vec![
			TxOut { value: 2324, script_pubkey: Script::from(monitored_pubkey.clone()) },
			TxOut { value: 0, script_pubkey: Script::from(monitored_pubkey.clone()) },
		])];
		let monitored_script_pubkeys = BTreeSet::from([monitored_pubkey]);
		let interesting_utxos = filter_interesting_utxos(txs, &monitored_script_pubkeys);
		assert_eq!(interesting_utxos.len(), 1);
		assert_eq!(interesting_utxos[0].value, 2324);
	}
}