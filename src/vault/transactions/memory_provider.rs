use super::{Liquidity, TransactionProvider};
use crate::{
    common::{coins::PoolCoin, Coin},
    side_chain::{ISideChain, SideChainTx},
    transactions::{QuoteTx, StakeQuoteTx, StakeTx, WitnessTx},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Transaction plus a boolean flag
pub struct FulfilledTxWrapper<Q> {
    /// The actual transaction
    pub inner: Q,
    /// Whether the transaction has been fulfilled (i.e. there
    /// is a matching "outcome" tx on the side chain)
    pub fulfilled: bool,
}

impl<Q> FulfilledTxWrapper<Q> {
    /// Constructor
    pub fn new(inner: Q, fulfilled: bool) -> FulfilledTxWrapper<Q> {
        FulfilledTxWrapper { inner, fulfilled }
    }
}

/// Witness transaction plus a boolean flag
pub struct WitnessTxWrapper {
    /// The actual transaction
    pub inner: WitnessTx,
    /// Whether the transaction has been used to fulfill
    /// some quote transaction
    pub used: bool,
}

impl WitnessTxWrapper {
    /// Construct from internal parts
    pub fn new(inner: WitnessTx, used: bool) -> Self {
        WitnessTxWrapper { inner, used }
    }
}

/// An in-memory transaction provider
pub struct MemoryTransactionsProvider<S: ISideChain> {
    side_chain: Arc<Mutex<S>>,
    quote_txs: Vec<FulfilledTxWrapper<QuoteTx>>,
    stake_quote_txs: Vec<FulfilledTxWrapper<StakeQuoteTx>>,
    stake_txs: Vec<StakeTx>,
    witness_txs: Vec<WitnessTxWrapper>,
    pools: HashMap<Coin, Liquidity>,
    next_block_idx: u32,
}

impl<S: ISideChain> MemoryTransactionsProvider<S> {
    /// Create an in-memory transaction provider
    pub fn new(side_chain: Arc<Mutex<S>>) -> Self {
        MemoryTransactionsProvider {
            side_chain: side_chain,
            quote_txs: vec![],
            stake_quote_txs: vec![],
            stake_txs: vec![],
            witness_txs: vec![],
            pools: HashMap::new(),
            next_block_idx: 0,
        }
    }
}

impl<S: ISideChain> TransactionProvider for MemoryTransactionsProvider<S> {
    fn sync(&mut self) -> u32 {
        let side_chain = self.side_chain.lock().unwrap();
        while let Some(block) = side_chain.get_block(self.next_block_idx) {
            debug!("TX Provider processing block: {}", self.next_block_idx);

            for tx in block.clone().txs {
                match tx {
                    SideChainTx::QuoteTx(tx) => {
                        // Quote transactions always come before their
                        // corresponding "outcome" tx, so they start unfulfilled
                        let tx = FulfilledTxWrapper::new(tx, false);

                        self.quote_txs.push(tx);
                    }
                    SideChainTx::StakeQuoteTx(tx) => {
                        // (same as above)
                        let tx = FulfilledTxWrapper::new(tx, false);

                        self.stake_quote_txs.push(tx)
                    }
                    SideChainTx::WitnessTx(tx) => {
                        // We assume that witness transactions arrive unused
                        let tx = WitnessTxWrapper {
                            inner: tx,
                            used: false,
                        };

                        self.witness_txs.push(tx);
                    }
                    SideChainTx::PoolChangeTx(tx) => {
                        let mut liquidity = self
                            .pools
                            .get(&tx.coin.get_coin())
                            .cloned()
                            .unwrap_or(Liquidity::new());

                        warn!("Pool change tx: {:?}, liquidity: {:?}", &tx, &liquidity);

                        let depth = liquidity.depth as i128 + tx.depth_change;
                        let loki_depth = liquidity.loki_depth as i128 + tx.loki_depth_change;
                        if depth < 0 || loki_depth < 0 {
                            error!(
                                "Negative liquidity depth found for tx: {:?}. Current: {:?}",
                                tx, liquidity
                            );
                            panic!("Negative liquidity depth found");
                        }

                        liquidity.depth = depth as u128;
                        liquidity.loki_depth = loki_depth as u128;
                        self.pools.insert(tx.coin.get_coin(), liquidity);
                    }
                    SideChainTx::StakeTx(tx) => {
                        // Find quote and mark it as fulfilled
                        if let Some(quote_info) = self
                            .stake_quote_txs
                            .iter_mut()
                            .find(|quote_info| quote_info.inner.id == tx.quote_tx)
                        {
                            quote_info.fulfilled = true;
                        }

                        // Find witness transacitons and mark them as used:
                        for wtx_id in &tx.witness_txs {
                            if let Some(witness_info) =
                                self.witness_txs.iter_mut().find(|w| &w.inner.id == wtx_id)
                            {
                                witness_info.used = true;
                            }
                        }

                        self.stake_txs.push(tx)
                    }
                }
            }
            self.next_block_idx += 1;
        }

        self.next_block_idx
    }

    fn add_transactions(&mut self, txs: Vec<SideChainTx>) -> Result<(), String> {
        // Filter out any duplicate transactions
        let valid_txs: Vec<SideChainTx> = txs
            .into_iter()
            .filter(|tx| {
                if let SideChainTx::WitnessTx(tx) = tx {
                    return !self.witness_txs.iter().any(|witness| tx == &witness.inner);
                }

                true
            })
            .collect();

        if valid_txs.len() > 0 {
            self.side_chain.lock().unwrap().add_block(valid_txs)?;
        }

        self.sync();
        Ok(())
    }

    fn get_quote_txs(&self) -> &[FulfilledTxWrapper<QuoteTx>] {
        &self.quote_txs
    }

    fn get_stake_quote_txs(&self) -> &[FulfilledTxWrapper<StakeQuoteTx>] {
        &self.stake_quote_txs
    }

    fn get_witness_txs(&self) -> &[WitnessTxWrapper] {
        &self.witness_txs
    }

    fn get_liquidity(&self, pool: PoolCoin) -> Option<Liquidity> {
        self.pools.get(&pool.get_coin()).cloned()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::side_chain::MemorySideChain;
    use crate::{transactions::PoolChangeTx, utils::test_utils::create_fake_quote_tx};
    use uuid::Uuid;

    fn setup() -> MemoryTransactionsProvider<MemorySideChain> {
        let side_chain = Arc::new(Mutex::new(MemorySideChain::new()));
        MemoryTransactionsProvider::new(side_chain)
    }

    #[test]
    fn test_provider() {
        let mut provider = setup();

        assert!(provider.get_quote_txs().is_empty());
        assert!(provider.get_witness_txs().is_empty());

        // Add some random blocks
        {
            let mut side_chain = provider.side_chain.lock().unwrap();

            let quote = create_fake_quote_tx();
            let witness = WitnessTx {
                id: Uuid::new_v4(),
                quote_id: quote.id,
                transaction_id: "0".to_owned(),
                transaction_block_number: 0,
                transaction_index: 1,
                amount: 100,
                coin_type: Coin::ETH,
                sender: None,
            };

            side_chain
                .add_block(vec![quote.into(), witness.into()])
                .unwrap();
        }

        provider.sync();

        assert_eq!(provider.next_block_idx, 1);
        assert_eq!(provider.get_quote_txs().len(), 1);
        assert_eq!(provider.get_witness_txs().len(), 1);

        provider
            .add_transactions(vec![create_fake_quote_tx().into()])
            .unwrap();

        assert_eq!(provider.next_block_idx, 2);
        assert_eq!(provider.get_quote_txs().len(), 2);
    }

    #[test]
    fn test_provider_does_not_add_duplicates() {
        let mut provider = setup();

        let quote = create_fake_quote_tx();
        let witness = WitnessTx {
            id: Uuid::new_v4(),
            quote_id: quote.id,
            transaction_id: "0".to_owned(),
            transaction_block_number: 0,
            transaction_index: 1,
            amount: 100,
            coin_type: Coin::ETH,
            sender: None,
        };

        {
            let mut side_chain = provider.side_chain.lock().unwrap();

            side_chain
                .add_block(vec![quote.into(), witness.clone().into()])
                .unwrap();
        }

        provider.sync();

        assert_eq!(provider.get_witness_txs().len(), 1);
        assert_eq!(provider.next_block_idx, 1);

        provider.add_transactions(vec![witness.into()]).unwrap();

        assert_eq!(provider.get_witness_txs().len(), 1);
        assert_eq!(provider.next_block_idx, 1);
    }

    #[test]
    #[should_panic(expected = "Negative liquidity depth found")]
    fn test_provider_panics_on_negative_liquidity() {
        let coin = PoolCoin::from(Coin::ETH).expect("Expected valid pool coin");
        let mut provider = setup();
        {
            let change_tx = PoolChangeTx::new(coin, -100, -100);

            let mut side_chain = provider.side_chain.lock().unwrap();

            side_chain.add_block(vec![change_tx.into()]).unwrap();
        }

        // Pre condition check
        assert!(provider.get_liquidity(coin).is_none());

        provider.sync();
    }

    #[test]
    fn test_provider_tallies_liquidity() {
        let coin = PoolCoin::from(Coin::ETH).expect("Expected valid pool coin");
        let mut provider = setup();
        {
            let mut side_chain = provider.side_chain.lock().unwrap();

            side_chain
                .add_block(vec![
                    PoolChangeTx::new(coin, 100, 100).into(),
                    PoolChangeTx::new(coin, -50, 100).into(),
                ])
                .unwrap();
        }

        assert!(provider.get_liquidity(coin).is_none());

        provider.sync();

        let liquidity = provider
            .get_liquidity(coin)
            .expect("Expected liquidity to exist");

        assert_eq!(liquidity.depth, 200);
        assert_eq!(liquidity.loki_depth, 50);
    }
}
