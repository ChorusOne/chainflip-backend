use std::collections::HashMap;

use pallet_cf_vaults::CeremonyId;

use crate::multisig::client;

use client::{
    common::{
        broadcast::{verify_broadcasts, BroadcastStage, BroadcastStageProcessor, DataToSend},
        CeremonyCommon, KeygenResult, StageResult,
    },
    keygen, ThresholdParameters,
};

use crate::multisig::crypto::{BigInt, BigIntConverter, ECPoint, KeyShare};

use keygen::{
    keygen_data::{
        BlameResponse6, Comm1, Complaints4, KeygenData, SecretShare3, VerifyComm2,
        VerifyComplaints5,
    },
    keygen_frost::{
        derive_aggregate_pubkey, derive_local_pubkeys_for_parties, generate_keygen_context,
        generate_shares_and_commitment, validate_commitments, verify_share, DKGCommitment,
        DKGUnverifiedCommitment, IncomingShares, OutgoingShares,
    },
    KeygenP2PSender,
};

use super::keygen_data::VerifyBlameResponses7;
use super::KeygenOptions;

type KeygenCeremonyCommon = CeremonyCommon<KeygenData, KeygenP2PSender>;

/// Stage 1: Sample a secret, generate sharing polynomial coefficients for it
/// and a ZKP of the secret. Broadcast commitments to the coefficients and the ZKP.
#[derive(Clone)]
pub struct AwaitCommitments1 {
    common: KeygenCeremonyCommon,
    own_commitment: DKGUnverifiedCommitment,
    shares: OutgoingShares,
    keygen_options: KeygenOptions,
}

impl AwaitCommitments1 {
    pub fn new(
        ceremony_id: CeremonyId,
        common: KeygenCeremonyCommon,
        keygen_options: KeygenOptions,
    ) -> Self {
        let params = ThresholdParameters::from_share_count(common.all_idxs.len());

        let context = generate_keygen_context(ceremony_id);

        let (shares, own_commitment) =
            generate_shares_and_commitment(&context, common.own_idx, params);

        AwaitCommitments1 {
            common,
            own_commitment,
            shares,
            keygen_options,
        }
    }
}

derive_display_as_type_name!(AwaitCommitments1);

impl BroadcastStageProcessor<KeygenData, KeygenResult> for AwaitCommitments1 {
    type Message = Comm1;

    fn init(&self) -> DataToSend<Self::Message> {
        DataToSend::Broadcast(self.own_commitment.clone())
    }

    fn should_delay(&self, m: &KeygenData) -> bool {
        matches!(m, KeygenData::Verify2(_))
    }

    fn process(
        self,
        messages: HashMap<usize, Self::Message>,
    ) -> StageResult<KeygenData, KeygenResult> {
        // We have received commitments from everyone, for now just need to
        // go through another round to verify consistent broadcasts

        let processor = VerifyCommitmentsBroadcast2 {
            common: self.common.clone(),
            commitments: messages,
            shares_to_send: self.shares,
            keygen_options: self.keygen_options,
        };

        let stage = BroadcastStage::new(processor, self.common);

        StageResult::NextStage(Box::new(stage))
    }
}

/// Stage 2: verify broadcasts of Stage 1 data
#[derive(Clone)]
struct VerifyCommitmentsBroadcast2 {
    common: KeygenCeremonyCommon,
    commitments: HashMap<usize, Comm1>,
    shares_to_send: OutgoingShares,
    keygen_options: KeygenOptions,
}

derive_display_as_type_name!(VerifyCommitmentsBroadcast2);

/// Check if the public key's x coordinate is smaller than "half secp256k1's order",
/// which is a requirement imposed by the Key Manager contract
fn is_contract_compatible(pk: &secp256k1::PublicKey) -> bool {
    let pubkey = cf_chains::eth::AggKey::from(pk);

    let x = BigInt::from_bytes(&pubkey.pub_key_x);
    let half_order = BigInt::from_bytes(&secp256k1::constants::CURVE_ORDER) / 2 + 1;

    x < half_order
}

impl BroadcastStageProcessor<KeygenData, KeygenResult> for VerifyCommitmentsBroadcast2 {
    type Message = VerifyComm2;

    fn init(&self) -> DataToSend<Self::Message> {
        let data = self.commitments.clone();

        DataToSend::Broadcast(VerifyComm2 { data })
    }

    fn should_delay(&self, m: &KeygenData) -> bool {
        matches!(m, KeygenData::SecretShares3(_))
    }

    fn process(
        self,
        messages: std::collections::HashMap<usize, Self::Message>,
    ) -> StageResult<KeygenData, KeygenResult> {
        let commitments = match verify_broadcasts(&messages) {
            Ok(comms) => comms,
            Err(blamed_parties) => return StageResult::Error(blamed_parties),
        };

        let context = generate_keygen_context(self.common.ceremony_id);

        let commitments = match validate_commitments(commitments, &context) {
            Ok(comms) => comms,
            Err(blamed_parties) => return StageResult::Error(blamed_parties),
        };

        slog::debug!(
            self.common.logger,
            "Initial commitments have been correctly broadcast"
        );

        // At this point we know everyone's commitments, which can already be
        // used to derive the resulting aggregate public key. Before proceeding
        // with the ceremony, we need to make sure that the key is compatible
        // with the Key Manager contract, aborting if it isn't.

        let agg_pubkey = derive_aggregate_pubkey(&commitments);

        // Note that we skip this check in tests as it would make them
        // non-deterministic (in the future, we could address this by
        // making the signer use deterministic randomness everywhere)
        if !self.keygen_options.low_pubkey_only || is_contract_compatible(&agg_pubkey.get_element())
        {
            let processor = SecretSharesStage3 {
                common: self.common.clone(),
                commitments,
                shares: self.shares_to_send,
            };

            let stage = BroadcastStage::new(processor, self.common);

            StageResult::NextStage(Box::new(stage))
        } else {
            slog::debug!(
                self.common.logger,
                "The key is not contract compatible, aborting..."
            );
            // It is nobody's fault that the key is not compatible,
            // so we abort with an empty list of responsible nodes
            // to let the State Chain restart the ceremony
            StageResult::Error(vec![])
        }
    }
}

/// Stage 3: distribute (distinct) secret shares of our secret to each party
#[derive(Clone)]
struct SecretSharesStage3 {
    common: KeygenCeremonyCommon,
    // commitments (verified to have been broadcast correctly)
    commitments: HashMap<usize, DKGCommitment>,
    shares: OutgoingShares,
}

derive_display_as_type_name!(SecretSharesStage3);

impl BroadcastStageProcessor<KeygenData, KeygenResult> for SecretSharesStage3 {
    type Message = SecretShare3;

    fn init(&self) -> DataToSend<Self::Message> {
        // With everyone committed to their secrets and sharing polynomial coefficients
        // we can now send the *distinct* secret shares to each party

        DataToSend::Private(self.shares.0.clone())
    }

    fn should_delay(&self, m: &KeygenData) -> bool {
        matches!(m, KeygenData::Complaints4(_))
    }

    fn process(
        self,
        incoming_shares: HashMap<usize, Self::Message>,
    ) -> StageResult<KeygenData, KeygenResult> {
        // As the messages for this stage are sent in secret, it is possible
        // for a malicious party to send us invalid data without us being able to prove
        // that. Because of that, we can't simply terminate our protocol here.

        let bad_parties: Vec<_> = incoming_shares
            .iter()
            .filter_map(|(sender_idx, share)| {
                if verify_share(share, &self.commitments[sender_idx], self.common.own_idx) {
                    None
                } else {
                    slog::warn!(
                        self.common.logger,
                        "Received invalid secret share from party: {}",
                        sender_idx
                    );
                    Some(*sender_idx)
                }
            })
            .collect();

        let processor = ComplaintsStage4 {
            common: self.common.clone(),
            commitments: self.commitments,
            shares: IncomingShares(incoming_shares),
            outgoing_shares: self.shares,
            complaints: bad_parties,
        };
        let stage = BroadcastStage::new(processor, self.common);

        StageResult::NextStage(Box::new(stage))
    }
}

/// During this stage parties have a chance to complain about
/// a party sending a secret share that isn't valid when checked
/// against the commitments
#[derive(Clone)]
struct ComplaintsStage4 {
    common: KeygenCeremonyCommon,
    // commitments (verified to have been broadcast correctly)
    commitments: HashMap<usize, DKGCommitment>,
    shares: IncomingShares,
    outgoing_shares: OutgoingShares,
    complaints: Vec<usize>,
}

derive_display_as_type_name!(ComplaintsStage4);

impl BroadcastStageProcessor<KeygenData, KeygenResult> for ComplaintsStage4 {
    type Message = Complaints4;

    fn init(&self) -> DataToSend<Self::Message> {
        DataToSend::Broadcast(Complaints4(self.complaints.clone()))
    }

    fn should_delay(&self, m: &KeygenData) -> bool {
        matches!(m, KeygenData::VerifyComplaints5(_))
    }

    fn process(
        self,
        messages: HashMap<usize, Self::Message>,
    ) -> StageResult<KeygenData, KeygenResult> {
        let processor = VerifyComplaintsBroadcastStage5 {
            common: self.common.clone(),
            received_complaints: messages,
            commitments: self.commitments,
            shares: self.shares,
            outgoing_shares: self.outgoing_shares,
        };

        let stage = BroadcastStage::new(processor, self.common);

        StageResult::NextStage(Box::new(stage))
    }
}

#[derive(Clone)]
struct VerifyComplaintsBroadcastStage5 {
    common: KeygenCeremonyCommon,
    received_complaints: HashMap<usize, Complaints4>,
    commitments: HashMap<usize, DKGCommitment>,
    shares: IncomingShares,
    outgoing_shares: OutgoingShares,
}

derive_display_as_type_name!(VerifyComplaintsBroadcastStage5);

impl BroadcastStageProcessor<KeygenData, KeygenResult> for VerifyComplaintsBroadcastStage5 {
    type Message = VerifyComplaints5;

    fn init(&self) -> DataToSend<Self::Message> {
        let data = self.received_complaints.clone();

        DataToSend::Broadcast(VerifyComplaints5 { data })
    }

    fn should_delay(&self, data: &KeygenData) -> bool {
        matches!(data, KeygenData::BlameResponse6(_))
    }

    fn process(
        self,
        messages: HashMap<usize, Self::Message>,
    ) -> StageResult<KeygenData, KeygenResult> {
        let verified_complaints = match verify_broadcasts(&messages) {
            Ok(comms) => comms,
            Err(blamed_parties) => {
                return StageResult::Error(blamed_parties);
            }
        };

        if verified_complaints.iter().all(|(_idx, c)| c.0.is_empty()) {
            // if all complaints are empty, we can finalize the ceremony
            let keygen_result =
                compute_keygen_result(self.common.all_idxs.len(), &self.shares, &self.commitments);

            return StageResult::Done(keygen_result);
        };

        // Some complaints have been issued, entering the blaming stage

        let idxs_to_report: Vec<_> = verified_complaints
            .iter()
            .filter_map(|(idx_from, Complaints4(blamed_idxs))| {
                // Ensure that complaints contain valid, non-duplicate indexes
                let deduped_idxs = {
                    let mut idxs = blamed_idxs.clone();
                    idxs.sort_unstable();
                    idxs.dedup();
                    idxs
                };

                let has_duplicates = deduped_idxs.len() != blamed_idxs.len();

                if has_duplicates {
                    slog::warn!(
                        self.common.logger,
                        "Complaint had duplicates: {:?}",
                        blamed_idxs
                    );
                }

                let has_invalid_idxs = !blamed_idxs.iter().all(|idx_blamed| {
                    if self.common.is_idx_valid(*idx_blamed) {
                        true
                    } else {
                        slog::warn!(
                            self.common.logger,
                            "Invalid index in complaint: {:?}",
                            idx_blamed
                        );
                        false
                    }
                });

                if has_duplicates || has_invalid_idxs {
                    Some(*idx_from)
                } else {
                    None
                }
            })
            .collect();

        if idxs_to_report.is_empty() {
            let processor = BlameResponsesStage6 {
                common: self.common.clone(),
                complaints: verified_complaints,
                shares: self.shares,
                outgoing_shares: self.outgoing_shares,
                commitments: self.commitments,
            };

            let stage = BroadcastStage::new(processor, self.common);

            StageResult::NextStage(Box::new(stage))
        } else {
            StageResult::Error(idxs_to_report)
        }
    }
}

fn compute_keygen_result(
    share_count: usize,
    secret_shares: &IncomingShares,
    commitments: &HashMap<usize, DKGCommitment>,
) -> KeygenResult {
    let key_share = secret_shares
        .0
        .values()
        .into_iter()
        .map(|share| share.value)
        .reduce(|acc, share| acc + share)
        .expect("shares should be non-empty");

    // TODO: delete all received shares

    let agg_pubkey = derive_aggregate_pubkey(commitments);

    let params = ThresholdParameters::from_share_count(share_count);

    let party_public_keys = derive_local_pubkeys_for_parties(params, commitments);

    KeygenResult {
        key_share: KeyShare {
            y: agg_pubkey,
            x_i: key_share,
        },
        party_public_keys,
    }
}

#[derive(Clone)]
struct BlameResponsesStage6 {
    common: KeygenCeremonyCommon,
    complaints: HashMap<usize, Complaints4>,
    shares: IncomingShares,
    outgoing_shares: OutgoingShares,
    commitments: HashMap<usize, DKGCommitment>,
}

derive_display_as_type_name!(BlameResponsesStage6);

impl BroadcastStageProcessor<KeygenData, KeygenResult> for BlameResponsesStage6 {
    type Message = BlameResponse6;

    fn init(&self) -> DataToSend<Self::Message> {
        // Indexes at which to reveal/broadcast secret shares
        let idxs_to_reveal: Vec<_> = self
            .complaints
            .iter()
            .filter_map(|(idx, complaint)| {
                if complaint.0.contains(&self.common.own_idx) {
                    slog::warn!(
                        self.common.logger,
                        "[{}] we are blamed by [{}]",
                        self.common.own_idx,
                        idx
                    );

                    Some(*idx)
                } else {
                    None
                }
            })
            .collect();

        // TODO: put a limit on how many shares to reveal?
        DataToSend::Broadcast(BlameResponse6(
            idxs_to_reveal
                .iter()
                .map(|idx| {
                    slog::debug!(self.common.logger, "revealing share for [{}]", *idx);
                    (*idx, self.outgoing_shares.0[idx].clone())
                })
                .collect(),
        ))
    }

    fn should_delay(&self, data: &KeygenData) -> bool {
        matches!(data, KeygenData::VerifyBlameResponses7(_))
    }

    fn process(
        self,
        blame_responses: HashMap<usize, Self::Message>,
    ) -> StageResult<KeygenData, KeygenResult> {
        // verify broadcasts of blame responses

        let processor = VerifyBlameResponsesBroadcastStage7 {
            common: self.common.clone(),
            blame_responses,
            shares: self.shares,
            commitments: self.commitments,
        };

        let stage = BroadcastStage::new(processor, self.common);

        StageResult::NextStage(Box::new(stage))
    }
}

#[derive(Clone)]
struct VerifyBlameResponsesBroadcastStage7 {
    common: KeygenCeremonyCommon,
    blame_responses: HashMap<usize, BlameResponse6>,
    shares: IncomingShares,
    commitments: HashMap<usize, DKGCommitment>,
}

derive_display_as_type_name!(VerifyBlameResponsesBroadcastStage7);

impl BroadcastStageProcessor<KeygenData, KeygenResult> for VerifyBlameResponsesBroadcastStage7 {
    type Message = VerifyBlameResponses7;

    fn init(&self) -> DataToSend<Self::Message> {
        let data = self.blame_responses.clone();

        DataToSend::Broadcast(VerifyBlameResponses7 { data })
    }

    fn should_delay(&self, _: &KeygenData) -> bool {
        false
    }

    fn process(
        mut self,
        messages: HashMap<usize, Self::Message>,
    ) -> StageResult<KeygenData, KeygenResult> {
        slog::debug!(
            self.common.logger,
            "Processing verifications for blame responses"
        );

        let verified_responses = match verify_broadcasts(&messages) {
            Ok(comms) => comms,
            Err(blamed_parties) => {
                return StageResult::Error(blamed_parties);
            }
        };

        let mut bad_parties = vec![];

        for (sender_idx, response) in verified_responses {
            for (dest_idx, share) in response.0 {
                let commitment = &self.commitments[&sender_idx];

                if verify_share(&share, commitment, dest_idx) {
                    // if the share is meant for us, save it
                    if dest_idx == self.common.own_idx {
                        // Sanity check: we shouldn't have complained about this share
                        // if it was valid to begin with:
                        debug_assert!(share.value != self.shares.0[&sender_idx].value);
                        self.shares.0.insert(sender_idx, share);
                    }
                } else {
                    slog::warn!(
                        self.common.logger,
                        "[{}] Invalid secret share in a blame response from party: {}",
                        self.common.own_idx,
                        sender_idx
                    );

                    bad_parties.push(sender_idx);
                }
            }
        }

        if bad_parties.is_empty() {
            let keygen_result =
                compute_keygen_result(self.common.all_idxs.len(), &self.shares, &self.commitments);

            StageResult::Done(keygen_result)
        } else {
            StageResult::Error(bad_parties)
        }
    }
}
