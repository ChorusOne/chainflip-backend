use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::p2p::AccountId;

use super::client_inner::Error;
use crate::signing::{MessageInfo, SigningOutcome};

use super::client_inner::{CeremonyOutcomeResult, MultisigMessage};

use super::common::{
    broadcast::{BroadcastStage, MessageWrapper},
    CeremonyCommon, CeremonyStage, KeygenResult, P2PSender, ProcessMessageResult, StageResult,
};

use super::frost::{SigningData, SigningDataWrapped};
use super::utils::ValidatorMaps;
use super::{InnerEvent, KeygenResultInfo, SchnorrSignature};

use super::frost_stages::AwaitCommitments1;

type EventSender = mpsc::UnboundedSender<InnerEvent>;

dyn_clone::clone_trait_object!(CeremonyStage<Message = SigningData, Result = SchnorrSignature>);

#[derive(Clone)]
pub struct SigningMessageWrapper {
    message_info: MessageInfo,
}

impl SigningMessageWrapper {
    pub fn new(message_info: MessageInfo) -> Self {
        SigningMessageWrapper { message_info }
    }
}

impl MessageWrapper<SigningData> for SigningMessageWrapper {
    fn wrap_and_serialize(&self, data: &SigningData) -> Vec<u8> {
        // add message info to data
        let msg: MultisigMessage =
            SigningDataWrapped::new(data.clone(), self.message_info.clone()).into();

        bincode::serialize(&msg).unwrap()
    }
}

#[derive(Clone)]
struct SigningStatePreKey {
    /// We need to store senders as `AccountId` as we might
    /// not know the
    delayed_messages: Vec<(AccountId, SigningData)>,
    should_expire_at: std::time::Instant,
}

#[derive(Clone)]
struct SigningStateWithKey {
    state: Option<Box<dyn CeremonyStage<Message = SigningData, Result = SchnorrSignature>>>,
    delayed_messages: Vec<(usize, SigningData)>,
    // TODO: this should be specialized to sending
    // results only (no p2p stuff)
    result_sender: EventSender,
    message_info: MessageInfo,
    validator_map: Arc<ValidatorMaps>,
    should_expire_at: std::time::Instant,
}

#[derive(Clone)]
enum SigningStateInner {
    SigningStatePreKey(SigningStatePreKey),
    SigningStateWithKey(SigningStateWithKey),
}

/// State for a signing ceremony
#[derive(Clone)]
pub struct SigningState {
    inner: SigningStateInner,
}

impl SigningStatePreKey {
    fn add_delayed(&mut self, id: AccountId, m: SigningData) {
        println!("Adding a delayed message");
        self.delayed_messages.push((id, m));
    }

    fn try_expiring(&self) -> Option<Vec<AccountId>> {
        let now = Instant::now();

        if self.should_expire_at < now {
            let nodes = self
                .delayed_messages
                .iter()
                .map(|(id, _)| id.clone())
                .collect();
            Some(nodes)
        } else {
            None
        }
    }
}

impl SigningStateWithKey {
    fn process_message_for_idx(&mut self, sender_idx: usize, m: SigningData) {
        // We know it is safe to unwrap because the value is None
        // for a brief period of time when we swap states below
        let state = self.state.as_mut().unwrap();

        // TODO: check that the party is a signer for this ceremony
        if state.should_delay(&m) {
            println!("Delaying message {} from party idx [{}]", m, sender_idx);
            self.delayed_messages.push((sender_idx, m));
            return;
        }

        let res = state.process_message(sender_idx, m);

        match res {
            ProcessMessageResult::CollectedAll => {
                let state = self.state.take().unwrap();

                // Is this the only point at which we can get result?
                // (no, there is also timeout)
                match state.finalize() {
                    StageResult::NextStage(mut stage) => {
                        println!("State transition to {}", &stage);

                        stage.init();

                        self.state = Some(stage);

                        // NOTE: we don't care when the state transition
                        // actually happened as we don't want other parties
                        // to be able to influence when our stages time out
                        // (any remaining time carries over to the next stage)
                        self.should_expire_at += STAGE_DURATION;

                        self.process_delayed();

                        // TODO: Should delete this state
                    }
                    StageResult::Error(bad_validators) => {
                        let blamed_parties = bad_validators
                            .iter()
                            .map(|idx| self.validator_map.get_id(*idx).unwrap().clone())
                            .collect();

                        self.send_result(Err((Error::Invalid, blamed_parties)));
                    }
                    StageResult::Done(signature) => {
                        self.send_result(Ok(signature));

                        println!("Reached final stage!");
                    }
                }
            }
            ProcessMessageResult::Ignored | ProcessMessageResult::Progress => {
                // Nothing to do
            }
        }
    }

    fn process_message(&mut self, id: AccountId, m: SigningData) {
        // Check that the validator has access to key
        let sender_idx = match self.validator_map.get_idx(&id) {
            Some(idx) => idx,
            None => return,
        };

        self.process_message_for_idx(sender_idx, m)
    }

    fn process_delayed(&mut self) {
        let messages = self.delayed_messages.split_off(0);

        for (idx, m) in messages {
            println!("Processing delayed message {} from party [{}]", m, idx);
            self.process_message_for_idx(idx, m);
        }
    }

    fn try_expiring(&self) -> Option<Vec<AccountId>> {
        let now = Instant::now();

        if self.should_expire_at < now {
            let late_idxs = self.state.as_ref().unwrap().awaited_parties();

            let late_ids = late_idxs
                .iter()
                .map(|idx| self.validator_map.get_id(*idx).unwrap().clone())
                .collect();

            Some(late_ids)
        } else {
            None
        }
    }

    fn send_result(&self, result: CeremonyOutcomeResult<SchnorrSignature>) {
        self.result_sender
            .send(InnerEvent::SigningResult(SigningOutcome {
                ceremony_id: self.message_info.clone(),
                result,
            }))
            .unwrap();
    }
}

const STAGE_DURATION: Duration = Duration::from_secs(15);

impl SigningState {
    /// Upgrade existing state to authorised (with a key) if it isn't already,
    /// and process any delayed messages
    pub fn on_request_to_sign(
        &mut self,
        signer_idx: usize,
        signer_idxs: Vec<usize>,
        key_info: KeygenResultInfo,
        message_info: MessageInfo,
        event_sender: EventSender,
        logger: &slog::Logger,
    ) {
        println!("on_request to sign");

        let delayed_messages = match &mut self.inner {
            SigningStateInner::SigningStatePreKey(state) => state.delayed_messages.split_off(0),
            SigningStateInner::SigningStateWithKey(_) => {
                println!("Ignoring duplicate request to sign");
                return;
            }
        };

        let common = CeremonyCommon {
            p2p_sender: P2PSender::new(key_info.validator_map.clone(), event_sender.clone()),
            own_idx: signer_idx,
            all_idxs: signer_idxs.clone(),
        };

        let signing_common = SigningStateCommonInfo {
            message_info: message_info.clone(),
            key: key_info.key.clone(),
            logger: logger.clone(),
        };

        let processor = AwaitCommitments1::new(common.clone(), signing_common);

        let mut state = BroadcastStage::new(
            processor,
            common,
            SigningMessageWrapper::new(message_info.clone()),
        );

        state.init();

        let mut state = SigningStateWithKey {
            state: Some(Box::new(state)),
            validator_map: key_info.validator_map.clone(),
            delayed_messages: Vec::new(),
            result_sender: event_sender,
            message_info,
            // Unlike other state transitions, we don't take into account
            // any time left in the prior stage when receiving a request
            // to sign (we don't want other parties to be able to
            // control when our stages time out)
            should_expire_at: Instant::now() + STAGE_DURATION,
        };

        // process delayed messages
        for (id, m) in delayed_messages {
            state.process_message(id, m);
        }

        self.inner = SigningStateInner::SigningStateWithKey(state);
    }

    /// Create State w/o access to key info with
    /// the only purpose of being able to keep delayed
    /// messages in the same place
    pub fn new_unauthorised() -> Self {
        SigningState {
            inner: SigningStateInner::SigningStatePreKey(SigningStatePreKey {
                delayed_messages: Vec::new(),
                should_expire_at: Instant::now() + STAGE_DURATION,
            }),
        }
    }

    pub fn process_message(&mut self, id: AccountId, m: SigningData) {
        match &mut self.inner {
            SigningStateInner::SigningStatePreKey(state) => {
                state.add_delayed(id, m);
            }
            SigningStateInner::SigningStateWithKey(state) => {
                state.process_message(id, m);
            }
        }
    }

    /// Check expiration time, and report responsible nodes if expired
    pub fn try_expiring(&self) -> Option<Vec<AccountId>> {
        match &self.inner {
            SigningStateInner::SigningStatePreKey(state) => state.try_expiring(),
            SigningStateInner::SigningStateWithKey(state) => state.try_expiring(),
        }
    }

    #[cfg(test)]
    pub fn get_stage(&self) -> Option<String> {
        match &self.inner {
            SigningStateInner::SigningStatePreKey(_) => None,
            SigningStateInner::SigningStateWithKey(state) => {
                state.state.as_ref().map(|s| s.to_string())
            }
        }
    }
}

/// Info useful for most signing states
#[derive(Clone)]
pub struct SigningStateCommonInfo {
    pub(super) message_info: MessageInfo,
    pub(super) key: Arc<KeygenResult>,
    logger: slog::Logger,
}
