#[cfg(feature = "try-runtime")]
use crate::AccountId;
use crate::Runtime;
use frame_support::traits::GetStorageVersion;
use pallet_cf_funding::migrations::active_bidders_migration::APPLY_AT_FUNDING_STORAGE_VERSION;
use pallet_cf_validator::migrations::active_bidders_migration::APPLY_AT_VALIDATOR_STORAGE_VERSION;
use sp_std::collections::btree_set::BTreeSet;

#[cfg(feature = "try-runtime")]
use sp_std::vec::Vec;
pub struct Migration;

impl frame_support::traits::OnRuntimeUpgrade for Migration {
	fn on_runtime_upgrade() -> frame_support::weights::Weight {
		if <pallet_cf_funding::Pallet<Runtime> as GetStorageVersion>::on_chain_storage_version() ==
			APPLY_AT_FUNDING_STORAGE_VERSION &&
			<pallet_cf_validator::Pallet<Runtime> as GetStorageVersion>::on_chain_storage_version(
			) == APPLY_AT_VALIDATOR_STORAGE_VERSION
		{
			log::info!("🔨 Applying ActiveBidder migration.");

			let active_bidders =
				pallet_cf_funding::migrations::old::ActiveBidder::<Runtime>::iter()
					.filter_map(|(validator, is_bidding)| is_bidding.then_some(validator))
					.collect::<BTreeSet<_>>();

			pallet_cf_validator::ActiveBidder::<Runtime>::set(active_bidders)
		} else {
			log::info!(
				"⏭ Skipping ActiveBidder migration. Funding version: {:?}, Validator Version: {:?}",
				<pallet_cf_funding::Pallet<Runtime> as GetStorageVersion>::on_chain_storage_version(),
				<pallet_cf_validator::Pallet<Runtime> as GetStorageVersion>::on_chain_storage_version()
			);
		}
		Default::default()
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade() -> Result<Vec<u8>, sp_runtime::DispatchError> {
		use codec::Encode;
		use frame_support::migrations::VersionedPostUpgradeData;

		if <pallet_cf_funding::Pallet<Runtime> as GetStorageVersion>::on_chain_storage_version() <
			APPLY_AT_FUNDING_STORAGE_VERSION
		{
			Ok(VersionedPostUpgradeData::MigrationExecuted(
				pallet_cf_funding::migrations::old::ActiveBidder::<Runtime>::iter()
					.filter_map(|(validator, is_bidding)| is_bidding.then_some(validator))
					.collect::<BTreeSet<_>>()
					.encode(),
			)
			.encode())
		} else {
			Ok(VersionedPostUpgradeData::Noop.encode())
		}
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(state: Vec<u8>) -> Result<(), frame_support::sp_runtime::TryRuntimeError> {
		use codec::Decode;
		use frame_support::migrations::VersionedPostUpgradeData;

		if let VersionedPostUpgradeData::MigrationExecuted(pre_upgrade_data) =
			<VersionedPostUpgradeData>::decode(&mut &state[..])
				.map_err(|_| "Failed to decode pre-upgrade state.")?
		{
			let active_bidders = <BTreeSet<AccountId>>::decode(&mut &pre_upgrade_data[..])
				.map_err(|_| "Failed to decode ActiveBidders from pre-upgrade state.")?;

			frame_support::ensure!(
				active_bidders == pallet_cf_validator::ActiveBidder::<Runtime>::get(),
				"Pre-upgrade state does not match post-upgrade state for ActiveBidders."
			);
		}
		Ok(())
	}
}
