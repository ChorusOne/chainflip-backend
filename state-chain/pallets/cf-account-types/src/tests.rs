use crate::{mock::*, *};
use frame_support::traits::HandleLifetime;
use frame_system::Provider;

const ALICE: u64 = 1;
const BOB: u64 = 2;
const CHARLIE: u64 = 3;

#[test]
fn test_ensure_origin_struct() {
	new_test_ext().execute_with(|| {
		// Root and none should be invalid.
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::root()).unwrap_err();
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::none()).unwrap_err();
		EnsureValidator::<Test>::ensure_origin(OriginFor::<Test>::root()).unwrap_err();
		EnsureValidator::<Test>::ensure_origin(OriginFor::<Test>::none()).unwrap_err();
		EnsureLiquidityProvider::<Test>::ensure_origin(OriginFor::<Test>::root()).unwrap_err();
		EnsureLiquidityProvider::<Test>::ensure_origin(OriginFor::<Test>::none()).unwrap_err();

		// Validation should fail for non-existent accounts.
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(ALICE)).unwrap_err();
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(BOB)).unwrap_err();
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(CHARLIE)).unwrap_err();

		// Create the accounts.
		<Provider<Test> as HandleLifetime<u64>>::created(&ALICE).unwrap();
		<Provider<Test> as HandleLifetime<u64>>::created(&BOB).unwrap();
		<Provider<Test> as HandleLifetime<u64>>::created(&CHARLIE).unwrap();

		// Validation should fail for uninitalised accounts.
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(ALICE)).unwrap_err();
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(BOB)).unwrap_err();
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(CHARLIE)).unwrap_err();

		// Upgrade the accounts.
		Pallet::<Test>::register_as_relayer(&ALICE).unwrap();
		Pallet::<Test>::register_as_validator(&BOB).unwrap();
		Pallet::<Test>::register_as_liquidity_provider(&CHARLIE).unwrap();

		// Each account should validate as the correct account type and fail otherwise.
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(ALICE)).unwrap();
		EnsureValidator::<Test>::ensure_origin(OriginFor::<Test>::signed(ALICE)).unwrap_err();
		EnsureLiquidityProvider::<Test>::ensure_origin(OriginFor::<Test>::signed(ALICE))
			.unwrap_err();
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(BOB)).unwrap_err();
		EnsureValidator::<Test>::ensure_origin(OriginFor::<Test>::signed(BOB)).unwrap();
		EnsureLiquidityProvider::<Test>::ensure_origin(OriginFor::<Test>::signed(BOB)).unwrap_err();
		EnsureRelayer::<Test>::ensure_origin(OriginFor::<Test>::signed(CHARLIE)).unwrap_err();
		EnsureValidator::<Test>::ensure_origin(OriginFor::<Test>::signed(CHARLIE)).unwrap_err();
		EnsureLiquidityProvider::<Test>::ensure_origin(OriginFor::<Test>::signed(CHARLIE)).unwrap();
	});
}

#[test]
fn test_ensure_origin_fn() {
	new_test_ext().execute_with(|| {
		// Root and none should be invalid.
		ensure_relayer::<Test>(OriginFor::<Test>::root()).unwrap_err();
		ensure_relayer::<Test>(OriginFor::<Test>::none()).unwrap_err();
		ensure_validator::<Test>(OriginFor::<Test>::root()).unwrap_err();
		ensure_validator::<Test>(OriginFor::<Test>::none()).unwrap_err();
		ensure_liquidity_provider::<Test>(OriginFor::<Test>::root()).unwrap_err();
		ensure_liquidity_provider::<Test>(OriginFor::<Test>::none()).unwrap_err();

		// Validation should fail for non-existent accounts.
		ensure_relayer::<Test>(OriginFor::<Test>::signed(ALICE)).unwrap_err();
		ensure_relayer::<Test>(OriginFor::<Test>::signed(BOB)).unwrap_err();
		ensure_relayer::<Test>(OriginFor::<Test>::signed(CHARLIE)).unwrap_err();

		// Create the accounts.
		<Provider<Test> as HandleLifetime<u64>>::created(&ALICE).unwrap();
		<Provider<Test> as HandleLifetime<u64>>::created(&BOB).unwrap();
		<Provider<Test> as HandleLifetime<u64>>::created(&CHARLIE).unwrap();

		// Validation should fail for uninitalised accounts.
		ensure_relayer::<Test>(OriginFor::<Test>::signed(ALICE)).unwrap_err();
		ensure_relayer::<Test>(OriginFor::<Test>::signed(BOB)).unwrap_err();
		ensure_relayer::<Test>(OriginFor::<Test>::signed(CHARLIE)).unwrap_err();

		// Upgrade the accounts.
		Pallet::<Test>::register_as_relayer(&ALICE).unwrap();
		Pallet::<Test>::register_as_validator(&BOB).unwrap();
		Pallet::<Test>::register_as_liquidity_provider(&CHARLIE).unwrap();

		// Each account should validate as the correct account type and fail otherwise.
		ensure_relayer::<Test>(OriginFor::<Test>::signed(ALICE)).unwrap();
		ensure_validator::<Test>(OriginFor::<Test>::signed(ALICE)).unwrap_err();
		ensure_liquidity_provider::<Test>(OriginFor::<Test>::signed(ALICE)).unwrap_err();
		ensure_relayer::<Test>(OriginFor::<Test>::signed(BOB)).unwrap_err();
		ensure_validator::<Test>(OriginFor::<Test>::signed(BOB)).unwrap();
		ensure_liquidity_provider::<Test>(OriginFor::<Test>::signed(BOB)).unwrap_err();
		ensure_relayer::<Test>(OriginFor::<Test>::signed(CHARLIE)).unwrap_err();
		ensure_validator::<Test>(OriginFor::<Test>::signed(CHARLIE)).unwrap_err();
		ensure_liquidity_provider::<Test>(OriginFor::<Test>::signed(CHARLIE)).unwrap();
	});
}