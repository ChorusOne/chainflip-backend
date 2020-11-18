#[macro_use]
extern crate log;

use blockswap::{
    common::store::PersistentKVS,
    logging,
    side_chain::PeristentSideChain,
    utils::{address::generate_btc_address_from_index, bip44},
    vault::{
        api::APIServer,
        blockchain_connection::{BtcSPVClient, LokiConnection, LokiConnectionConfig, Web3Client},
        config::{NetType, VAULT_CONFIG},
        processor::{
            BtcOutputSender, EthOutputSender, LokiSender, OutputCoinProcessor, SideChainProcessor,
        },
        transactions::{MemoryTransactionsProvider, TransactionProvider},
        witness::{BtcSPVWitness, EthereumWitness, LokiWitness},
    },
};
use parking_lot::RwLock;

use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// Entry point for the Vault node binary. We should try to keep it as small as posible
/// and implement most of the core logic as part of the library (src/lib.rs). This way
/// of organising code works better with integration tests.
/// Ideally we would just parse commad line arguments here and call into the library.
fn main() {
    std::panic::set_hook(Box::new(|msg| {
        error!("Panicked with: {}", msg);
        std::process::exit(101); // Rust's panics use 101 by default
    }));

    logging::init("vault", None);

    // Create the vault config and ensure it's valid
    let vault_config = &VAULT_CONFIG;

    info!("Starting a _ Vault node");

    let s_chain = PeristentSideChain::open("blocks.db");
    let s_chain = Arc::new(Mutex::new(s_chain));

    let mut provider = MemoryTransactionsProvider::new(s_chain.clone());
    provider.sync();

    let provider = Arc::new(RwLock::new(provider));

    let config = LokiConnectionConfig {
        rpc_wallet_port: vault_config.loki.rpc.port,
    };

    let loki_connection = LokiConnection::new(config);
    let loki_block_receiver = loki_connection.start();

    let eth_client =
        Web3Client::url(&vault_config.eth.provider_url).expect("Failed to create web3 client");

    let btc_network = match &vault_config.net_type {
        NetType::Testnet => bitcoin::Network::Testnet,
        NetType::Mainnet => bitcoin::Network::Bitcoin,
    };

    let btc_config = &vault_config.btc;

    // all change should go to address at index 0
    let btc_root_address = generate_btc_address_from_index(
        &btc_config.master_root_key,
        0,
        true,
        bitcoin::AddressType::P2wpkh,
        &vault_config.net_type,
    )
    .expect("Could not generate bitcoin address for index 0");

    let btc_change_address = bitcoin::Address::from_str(&btc_root_address)
        .expect("Couldn't get bitcoin Address type from type &str");

    let btc = BtcSPVClient::new(
        btc_config.rpc_port,
        btc_config.rpc_user.clone(),
        btc_config.rpc_password.clone(),
        btc_network,
        btc_change_address,
    );

    // Witnesses
    let db_connection = rusqlite::Connection::open("blocks.db").expect("Could not open database");
    let kvs = Arc::new(Mutex::new(PersistentKVS::new(db_connection)));
    let loki_witness = LokiWitness::new(loki_block_receiver, provider.clone());
    let eth_witness = EthereumWitness::new(Arc::new(eth_client.clone()), provider.clone(), kvs);
    let btc_witness = BtcSPVWitness::new(Arc::new(btc.clone()), provider.clone());

    loki_witness.start();
    eth_witness.start();
    btc_witness.start();

    // Processor
    let db_connection = rusqlite::Connection::open("blocks.db").expect("Could not open database");
    let kvs = PersistentKVS::new(db_connection);
    let loki = LokiSender::new(vault_config.loki.rpc.clone());

    let eth_root_key = match bip44::RawKey::decode(&vault_config.eth.master_root_key) {
        Ok(key) => key,
        Err(_) => panic!("Failed to generate root key from eth master root key"),
    };
    let eth_sender = EthOutputSender::new(eth_client.clone(), provider.clone(), eth_root_key);

    let btc_root_key = match bip44::RawKey::decode(&vault_config.btc.master_root_key) {
        Ok(key) => key,
        Err(_) => panic!("Failed to generate root key from btc master root key"),
    };
    let btc_sender = BtcOutputSender::new(
        btc.clone(),
        provider.clone(),
        btc_root_key,
        vault_config.net_type,
    );

    let coin_processor = OutputCoinProcessor::new(loki, eth_sender, btc_sender);
    let processor = SideChainProcessor::new(provider.clone(), kvs, coin_processor);

    processor.start(None);

    // API
    // can be used to shutdown the server
    let (_tx, rx) = tokio::sync::oneshot::channel();
    APIServer::serve(&VAULT_CONFIG, s_chain, provider, rx);
}
