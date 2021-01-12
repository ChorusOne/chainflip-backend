use chainflip_common::types::coin::Coin;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use crate::quoter::StateProvider;

#[derive(Default, Clone)]
pub struct InputIdCache(Arc<Mutex<HashMap<Coin, BTreeSet<Vec<u8>>>>>);

impl InputIdCache {
    pub fn from_state<S: StateProvider>(state: Arc<Mutex<S>>) -> Self {
        let mut cache: HashMap<Coin, BTreeSet<Vec<u8>>> = HashMap::new();
        let state = state.lock().unwrap();

        for quote in state.get_swap_quotes() {
            cache
                .entry(quote.input)
                .or_insert(BTreeSet::new())
                .insert(quote.input_address_id);
        }

        for quote in state.get_deposit_quotes() {
            cache
                .entry(quote.pool)
                .or_insert(BTreeSet::new())
                .insert(quote.coin_input_address_id);

            cache
                .entry(Coin::LOKI)
                .or_insert(BTreeSet::new())
                .insert(quote.base_input_address_id);
        }

        Self(Arc::new(Mutex::new(cache)))
    }

    fn update_cache<F, R>(&self, coin: &Coin, mut f: F) -> R
    where
        F: FnMut(&mut BTreeSet<Vec<u8>>) -> R,
    {
        let mut cache = self.0.lock().unwrap();
        f(cache.entry(*coin).or_default())
    }

    pub fn remove(&self, coin: &Coin, address: &[u8]) -> bool {
        self.update_cache(coin, |cache| cache.remove(address))
    }

    pub fn generate_unique_input_address_id_with_rng<R: rand::Rng>(
        &self,
        input_coin: &Coin,
        rng: &mut R,
    ) -> Vec<u8> {
        let mut cache = self.0.lock().unwrap();
        let used_ids = cache.entry(*input_coin).or_insert(BTreeSet::new());

        loop {
            let id = match input_coin {
                // BTC and ETH have u32 indexes which we can derive an address through hd wallets
                Coin::BTC | Coin::ETH => rng.gen_range(5, u32::MAX).to_be_bytes().to_vec(),
                // LOKI has 8 random bytes which represent a payment id
                Coin::LOKI => rng.gen::<[u8; 8]>().to_vec(),
            };

            if used_ids.insert(id.clone()) {
                break id;
            }
        }
    }

    pub fn generate_unique_input_address_id(&self, input_coin: &Coin) -> Vec<u8> {
        self.generate_unique_input_address_id_with_rng(input_coin, &mut rand::thread_rng())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rand::{prelude::StdRng, SeedableRng};
    use std::collections::HashMap;

    #[test]
    fn generates_id_and_inserts_to_cache() {
        let mut rng = StdRng::seed_from_u64(0);
        let cache = InputIdCache::default();

        for coin in vec![Coin::LOKI, Coin::ETH, Coin::BTC] {
            cache.generate_unique_input_address_id_with_rng(&coin, &mut rng);
            assert_eq!(cache.0.lock().unwrap().get(&coin).unwrap().len(), 1);
        }
    }

    #[test]
    fn generates_unique_ids() {
        let seed = 0;
        let cache = InputIdCache::default();

        // The expected first ids generated by the rng with the given seed
        let mut first_ids = HashMap::new();
        first_ids.insert(Coin::ETH, vec![129, 245, 247, 179]);
        first_ids.insert(Coin::BTC, vec![129, 245, 247, 179]);
        first_ids.insert(Coin::LOKI, vec![178, 214, 168, 126, 192, 105, 52, 255]);

        // generate first ids
        for coin in vec![Coin::LOKI, Coin::ETH, Coin::BTC] {
            let mut rng = StdRng::seed_from_u64(seed);
            cache.generate_unique_input_address_id_with_rng(&coin, &mut rng);

            let expected = first_ids.get(&coin).unwrap();
            {
                let coin_cache = cache.0.lock().unwrap();
                let set = coin_cache.get(&coin).unwrap();
                assert_eq!(set.len(), 1);
                assert!(
                    set.contains(expected),
                    "Set doesn't contain expected value for {}. Expected: {:?}. Got: {:?}",
                    coin,
                    expected,
                    set
                );
            }
        }

        // Generate other ids
        for coin in vec![Coin::LOKI, Coin::ETH, Coin::BTC] {
            let mut rng = StdRng::seed_from_u64(seed);
            cache.generate_unique_input_address_id_with_rng(&coin, &mut rng);
            assert_eq!(cache.0.lock().unwrap().get(&coin).unwrap().len(), 2);
        }
    }
}
