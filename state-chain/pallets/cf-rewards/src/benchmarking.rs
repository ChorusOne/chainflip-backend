//! Benchmarking setup for pallet-template

use super::*;

use frame_benchmarking::{benchmarks, impl_benchmark_test_suite, whitelisted_caller};
use frame_system::RawOrigin;
use sp_std::{boxed::Box, vec, vec::Vec};

#[allow(unused)]
use crate::Module as FlipRewards;

benchmarks! {
	do_something {
		let s in 0 .. 100;
		let caller: T::AccountId = whitelisted_caller();
	}: _(RawOrigin::Signed(caller), s)
	verify {
		// assert_eq!(Something::<T>::get(), Some(s));
	}
}

impl_benchmark_test_suite!(FlipRewards, crate::mock::new_test_ext(), crate::mock::Test,);
