//! Configuration, utilities and helpers for the Chainflip runtime.
use super::{
	AccountId, Emissions, Flip, FlipBalance, Online, Reputation, Rewards, Runtime, System, Validator,
	Witnesser,
};
use crate::{EmergencyRotationPercentageTrigger, HeartbeatBlockInterval};
use cf_traits::{BondRotation, ChainflipAccount, ChainflipAccountState, ChainflipAccountStore,
	EmergencyRotation, EmissionsTrigger, EpochInfo, Heartbeat, Issuance, NetworkState,
	StakeHandler, StakeTransfer, VaultRotationHandler,
};
use frame_support::{debug, weights::Weight};
use pallet_cf_auction::{HandleStakes, VaultRotationEventHandler};
use pallet_cf_validator::EpochTransitionHandler;
use sp_std::cmp::min;
use sp_std::vec::Vec;

pub struct ChainflipEpochTransitions;

/// Trigger emissions on epoch transitions.
impl EpochTransitionHandler for ChainflipEpochTransitions {
	type ValidatorId = AccountId;
	type Amount = FlipBalance;

	fn on_new_epoch(new_validators: &[Self::ValidatorId], new_bond: Self::Amount) {
		// Process any outstanding emissions.
		<Emissions as EmissionsTrigger>::trigger_emissions();
		// Rollover the rewards.
		Rewards::rollover(new_validators).unwrap_or_else(|err| {
			debug::error!("Unable to process rewards rollover: {:?}!", err);
		});
		// Update the the bond of all validators for the new epoch
		<Flip as BondRotation>::update_validator_bonds(new_validators, new_bond);
		// Update the list of validators in reputation
		<Online as EpochTransitionHandler>::on_new_epoch(new_validators, new_bond);
		// Update the list of validators in the witnesser.
		<Witnesser as EpochTransitionHandler>::on_new_epoch(new_validators, new_bond)
	}
}

pub struct ChainflipStakeHandler;
impl StakeHandler for ChainflipStakeHandler {
	type ValidatorId = AccountId;
	type Amount = FlipBalance;

	fn stake_updated(validator_id: &Self::ValidatorId, new_total: Self::Amount) {
		HandleStakes::<Runtime>::stake_updated(validator_id, new_total);
	}
}

pub struct ChainflipVaultRotationHandler;
impl VaultRotationHandler for ChainflipVaultRotationHandler {
	type ValidatorId = AccountId;

	fn vault_rotation_aborted() {
		VaultRotationEventHandler::<Runtime>::vault_rotation_aborted();
	}

	fn penalise(bad_validators: &[Self::ValidatorId]) {
		VaultRotationEventHandler::<Runtime>::penalise(bad_validators);
	}
}

trait RewardDistribution {
	type EpochInfo: EpochInfo;
	type StakeTransfer: StakeTransfer;
	type ValidatorId;
	type FlipBalance;
	/// An implementation of the [Issuance] trait.
	type Issuance: Issuance;

	/// Distribute rewards
	fn distribute_rewards(backup_validators: &[&Self::ValidatorId]) -> Weight;
}

struct BackupValidatorEmissions;

impl RewardDistribution for BackupValidatorEmissions {
	type EpochInfo = Validator;
	type StakeTransfer = Flip;
	type ValidatorId = AccountId;
	type FlipBalance = FlipBalance;
	type Issuance = pallet_cf_flip::FlipIssuance<Runtime>;

	// This is called on each heartbeat interval
	// Would need to calculate emissions for the 150 blocks the heartbeat is
	// TODO These should be configurable items in the emission pallet: Block emissions for Validator and BV
	// TODO calculated in pallet and configurable with extrinsics

	fn distribute_rewards(backup_validators: &[&Self::ValidatorId]) -> Weight {
		// The current minimum active bid
		let minimum_active_bid = Self::EpochInfo::bond();
		// Our emission cap for this heartbeat interval
		let emissions_cap = Emissions::backup_validator_block_emissions() * HeartbeatBlockInterval::get() as u128;
		// We distribute backup rewards every heartbeat interval
		// These are the rewards for this epoch, we don't know the size of the epoch in blocks
		// so this needs to be weighted for 150 blocks
		let block_emissions = System::block_number() - Emissions::last_mint_block();
		let average_validator_reward =
			Rewards::rewards_due_each() * HeartbeatBlockInterval::get() as u128 / block_emissions as u128 ;

		let mut total_rewards = 0;

		let mut rewards: Vec<(&Self::ValidatorId, Self::FlipBalance)> = backup_validators
			.iter()
			.map(|backup_validator| {
				let backup_validator_stake =
					Self::StakeTransfer::stakeable_balance(*backup_validator);
				let reward_scaling_factor =
					min(1, (backup_validator_stake / minimum_active_bid) ^ 2);
				let reward = (reward_scaling_factor * average_validator_reward * 8) / 10;
				total_rewards += reward;
				(*backup_validator, reward)
			})
			.collect();

		// Cap if needed and mint the rewards to be distributed
		if total_rewards > emissions_cap {
			rewards = rewards
				.into_iter()
				.map(|(validator_id, reward)| {
					(validator_id, (reward * emissions_cap) / total_rewards)
				})
				.collect();
		}

		// Distribute rewards
		for (validator_id, reward) in rewards {
			Flip::settle(&validator_id, Self::Issuance::mint(reward).into());
		}

		0
	}
}

pub struct ChainflipHeartbeat;

impl Heartbeat for ChainflipHeartbeat {
	type ValidatorId = AccountId;

	fn heartbeat_submitted(validator_id: &Self::ValidatorId) -> Weight {
		<Reputation as Heartbeat>::heartbeat_submitted(validator_id)
	}

	fn on_heartbeat_interval(network_state: NetworkState<Self::ValidatorId>) -> Weight {
		// Reputation depends on heartbeats
		let mut weight = <Reputation as Heartbeat>::on_heartbeat_interval(network_state.clone());

		// We pay rewards to online backup validators on each heartbeat interval
		let backup_validators: Vec<&Self::ValidatorId> = network_state
			.online
			.iter()
			.filter(|account_id| {
				ChainflipAccountStore::<Runtime>::get(*account_id).state
					== ChainflipAccountState::Backup
			})
			.collect();

		BackupValidatorEmissions::distribute_rewards(&backup_validators);

		// Check the state of the network and if we are below the emergency rotation trigger
		// then issue an emergency rotation request
		if network_state.percentage_online() < EmergencyRotationPercentageTrigger::get() as u32 {
			weight += <Validator as EmergencyRotation>::request_emergency_rotation();
		}

		weight
	}
}
