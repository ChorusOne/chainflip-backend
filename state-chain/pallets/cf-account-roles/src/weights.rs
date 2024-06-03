
//! Autogenerated weights for pallet_cf_account_roles
//!
//! THIS FILE WAS AUTO-GENERATED USING THE SUBSTRATE BENCHMARK CLI VERSION 4.0.0-dev
//! DATE: 2024-06-03, STEPS: `20`, REPEAT: `10`, LOW RANGE: `[]`, HIGH RANGE: `[]`
//! WORST CASE MAP SIZE: `1000000`
//! HOSTNAME: `ip-172-31-0-210`, CPU: `Intel(R) Xeon(R) Platinum 8124M CPU @ 3.00GHz`
//! EXECUTION: , WASM-EXECUTION: Compiled, CHAIN: Some("dev-3"), DB CACHE: 1024

// Executed Command:
// ./chainflip-node
// benchmark
// pallet
// --pallet
// pallet_cf_account_roles
// --extrinsic
// *
// --output
// state-chain/pallets/cf-account-roles/src/weights.rs
// --steps=20
// --repeat=10
// --template=state-chain/chainflip-weight-template.hbs
// --chain=dev-3

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for pallet_cf_account_roles.
pub trait WeightInfo {
	fn set_vanity_name() -> Weight;
}

/// Weights for pallet_cf_account_roles using the Substrate node and recommended hardware.
pub struct PalletWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for PalletWeight<T> {
	/// Storage: `AccountRoles::VanityNames` (r:1 w:1)
	/// Proof: `AccountRoles::VanityNames` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	fn set_vanity_name() -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `546`
		//  Estimated: `2031`
		// Minimum execution time: 14_622_000 picoseconds.
		Weight::from_parts(14_976_000, 2031)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
}

// For backwards compatibility and tests
impl WeightInfo for () {
	/// Storage: `AccountRoles::VanityNames` (r:1 w:1)
	/// Proof: `AccountRoles::VanityNames` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	fn set_vanity_name() -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `546`
		//  Estimated: `2031`
		// Minimum execution time: 14_622_000 picoseconds.
		Weight::from_parts(14_976_000, 2031)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
}
