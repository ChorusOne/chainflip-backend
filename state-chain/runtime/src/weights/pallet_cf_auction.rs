//! Autogenerated weights for pallet_cf_auction
//!
//! THIS FILE WAS AUTO-GENERATED USING THE SUBSTRATE BENCHMARK CLI VERSION 3.0.0
//! DATE: 2021-09-15, STEPS: [50, ], REPEAT: 20, LOW RANGE: [], HIGH RANGE: []
//! EXECUTION: Some(Wasm), WASM-EXECUTION: Interpreted, CHAIN: None, DB CACHE: 128

// Executed Command:
// /Users/janborner/develop/chainflip/chainflip-backend/target/release/state-chain-node
// benchmark
// --extrinsic
// *
// --pallet
// pallet_cf_auction
// --output
// runtime/src/weights
// --execution=wasm
// --steps=50
// --repeat=20

#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::Weight};
use sp_std::marker::PhantomData;

/// Weight functions for pallet_cf_auction.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> pallet_cf_auction::WeightInfo for WeightInfo<T> {
	fn set_auction_size_range() -> Weight {
		(79_000_000 as Weight)
			.saturating_add(T::DbWeight::get().reads(1 as Weight))
			.saturating_add(T::DbWeight::get().writes(1 as Weight))
	}
}
