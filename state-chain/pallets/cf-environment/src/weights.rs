
//! Autogenerated weights for pallet_cf_environment
//!
//! THIS FILE WAS AUTO-GENERATED USING THE SUBSTRATE BENCHMARK CLI VERSION 4.0.0-dev
//! DATE: 2023-02-06, STEPS: `2`, REPEAT: 1, LOW RANGE: `[]`, HIGH RANGE: `[]`
//! HOSTNAME: `kylezs.localdomain`, CPU: `<UNKNOWN>`
//! EXECUTION: Some(Wasm), WASM-EXECUTION: Compiled, CHAIN: None, DB CACHE: 1024

// Executed Command:
// ./target/release/chainflip-node
// benchmark
// pallet
// --extrinsic
// *
// --pallet
// pallet_cf_environment
// --output
// state-chain/pallets/cf-environment/src/weights.rs
// --execution=wasm
// --steps=2
// --repeat=1
// --template=state-chain/chainflip-weight-template.hbs

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use sp_std::marker::PhantomData;

/// Weight functions needed for pallet_cf_environment.
pub trait WeightInfo {
	fn set_cfe_settings() -> Weight;
	fn update_supported_eth_assets() -> Weight;
	fn update_polkadot_runtime_version() -> Weight;
	fn update_safe_mode() -> Weight;
}

/// Weights for pallet_cf_environment using the Substrate node and recommended hardware.
pub struct PalletWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for PalletWeight<T> {
	// Storage: Environment CfeSettings (r:0 w:1)
	fn set_cfe_settings() -> Weight {
		// Minimum execution time: 17_000 nanoseconds.
		Weight::from_ref_time(17_000_000)
			.saturating_add(T::DbWeight::get().writes(1))
	}
	// Storage: Environment EthereumSupportedAssets (r:1 w:1)
	fn update_supported_eth_assets() -> Weight {
		// Minimum execution time: 24_000 nanoseconds.
		Weight::from_ref_time(24_000_000)
			.saturating_add(T::DbWeight::get().reads(1))
			.saturating_add(T::DbWeight::get().writes(1))
	}
	// Storage: Environment PolkadotRuntimeVersion (r:1 w:1)
	fn update_polkadot_runtime_version() -> Weight {
		// Minimum execution time: 20_000 nanoseconds.
		Weight::from_ref_time(20_000_000)
			.saturating_add(T::DbWeight::get().reads(1))
			.saturating_add(T::DbWeight::get().writes(1))
	}
	// Storage: Environment PolkadotRuntimeVersion (r:1 w:1)
	fn update_safe_mode() -> Weight {
		// Minimum execution time: 20_000 nanoseconds.
		Weight::from_ref_time(20_000_000)
			.saturating_add(T::DbWeight::get().reads(1))
			.saturating_add(T::DbWeight::get().writes(1))
	}
}

// For backwards compatibility and tests
impl WeightInfo for () {
	// Storage: Environment CfeSettings (r:0 w:1)
	fn set_cfe_settings() -> Weight {
		// Minimum execution time: 17_000 nanoseconds.
		Weight::from_ref_time(17_000_000)
			.saturating_add(RocksDbWeight::get().writes(1))
	}
	// Storage: Environment EthereumSupportedAssets (r:1 w:1)
	fn update_supported_eth_assets() -> Weight {
		// Minimum execution time: 24_000 nanoseconds.
		Weight::from_ref_time(24_000_000)
			.saturating_add(RocksDbWeight::get().reads(1))
			.saturating_add(RocksDbWeight::get().writes(1))
	}
	// Storage: Environment PolkadotRuntimeVersion (r:1 w:1)
	fn update_polkadot_runtime_version() -> Weight {
		// Minimum execution time: 20_000 nanoseconds.
		Weight::from_ref_time(20_000_000)
			.saturating_add(RocksDbWeight::get().reads(1))
			.saturating_add(RocksDbWeight::get().writes(1))
	}
	// Storage: Environment PolkadotRuntimeVersion (r:1 w:1)
	fn update_safe_mode() -> Weight {
		// Minimum execution time: 20_000 nanoseconds.
		Weight::from_ref_time(20_000_000)
			.saturating_add(RocksDbWeight::get().reads(1))
			.saturating_add(RocksDbWeight::get().writes(1))
	}
}
