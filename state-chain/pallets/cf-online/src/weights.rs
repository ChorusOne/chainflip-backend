//! Autogenerated weights for pallet_cf_online
//!
//! THIS FILE WAS AUTO-GENERATED USING THE SUBSTRATE BENCHMARK CLI VERSION 4.0.0-dev
//! DATE: 2022-02-07, STEPS: `20`, REPEAT: 10, LOW RANGE: `[]`, HIGH RANGE: `[]`
//! EXECUTION: Some(Wasm), WASM-EXECUTION: Compiled, CHAIN: None, DB CACHE: 128

// Executed Command:
// ./target/release/chainflip-node
// benchmark
// --extrinsic
// *
// --pallet
// pallet_cf_online
// --output
// state-chain/pallets/cf-online/src/weights.rs
// --execution=wasm
// --steps=20
// --repeat=10
// --template=state-chain/chainflip-weight-template.hbs


#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use sp_std::marker::PhantomData;

/// Weight functions needed for pallet_cf_online.
pub trait WeightInfo {
	fn heartbeat() -> Weight;
	fn submit_network_state() -> Weight;
	fn on_initialize_no_action() -> Weight;
}

/// Weights for pallet_cf_online using the Substrate node and recommended hardware.
pub struct PalletWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for PalletWeight<T> {
	// Storage: Online Nodes (r:1 w:1)
	// Storage: Reputation Reputations (r:1 w:1)
	fn heartbeat() -> Weight {
		#[allow(clippy::unnecessary_cast)]
		(23_000_000 as Weight)
			.saturating_add(T::DbWeight::get().reads(2 as Weight))
			.saturating_add(T::DbWeight::get().writes(2 as Weight))
	}
	// Storage: Validator Validators (r:1 w:0)
	// Storage: Online Nodes (r:1 w:0)
	// Storage: Reputation Reputations (r:1 w:1)
	// Storage: Reputation ReputationPointPenalty (r:1 w:0)
	// Storage: Flip SlashingRate (r:1 w:0)
	// Storage: Auction RemainingBidders (r:1 w:0)
	// Storage: Auction BackupGroupSize (r:1 w:0)
	fn submit_network_state() -> Weight {
		#[allow(clippy::unnecessary_cast)]
		(36_000_000 as Weight)
			.saturating_add(T::DbWeight::get().reads(7 as Weight))
			.saturating_add(T::DbWeight::get().writes(1 as Weight))
	}
	fn on_initialize_no_action() -> Weight {
		#[allow(clippy::unnecessary_cast)]
		(0 as Weight)
	}
}

// For backwards compatibility and tests
impl WeightInfo for () {
	// Storage: Online Nodes (r:1 w:1)
	// Storage: Reputation Reputations (r:1 w:1)
	fn heartbeat() -> Weight {
		#[allow(clippy::unnecessary_cast)]
		(23_000_000 as Weight)
			.saturating_add(RocksDbWeight::get().reads(2 as Weight))
			.saturating_add(RocksDbWeight::get().writes(2 as Weight))
	}
	// Storage: Validator Validators (r:1 w:0)
	// Storage: Online Nodes (r:1 w:0)
	// Storage: Reputation Reputations (r:1 w:1)
	// Storage: Reputation ReputationPointPenalty (r:1 w:0)
	// Storage: Flip SlashingRate (r:1 w:0)
	// Storage: Auction RemainingBidders (r:1 w:0)
	// Storage: Auction BackupGroupSize (r:1 w:0)
	fn submit_network_state() -> Weight {
		#[allow(clippy::unnecessary_cast)]
		(36_000_000 as Weight)
			.saturating_add(RocksDbWeight::get().reads(7 as Weight))
			.saturating_add(RocksDbWeight::get().writes(1 as Weight))
	}
	fn on_initialize_no_action() -> Weight {
		#[allow(clippy::unnecessary_cast)]
		(0 as Weight)
	}
}