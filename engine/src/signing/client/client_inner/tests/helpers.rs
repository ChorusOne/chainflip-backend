use std::{collections::HashMap, time::Duration};

use itertools::Itertools;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::signing::client::client_inner::frost;

use frost::{SigningData, SigningDataWrapped};

use crate::{
    logging,
    p2p::{AccountId, P2PMessage, P2PMessageCommand},
    signing::{
        client::{
            client_inner::{
                client_inner::{
                    Broadcast1, KeyGenMessageWrapped, KeygenData, MultisigMessage,
                    SchnorrSignature, Secret2,
                },
                common::KeygenResultInfo,
                keygen_state::KeygenStage,
                InnerEvent, KeygenOutcome, MultisigClient, SigningOutcome,
            },
            KeyId, KeygenInfo, MultisigInstruction,
        },
        crypto::Keys,
        db::KeyDBMock,
        MessageInfo,
    },
};

type MultisigClientNoDB = MultisigClient<KeyDBMock>;

use super::{KEY_ID, MESSAGE_HASH, MESSAGE_INFO, SIGNER_IDXS, SIGN_INFO};

type InnerEventReceiver = UnboundedReceiver<InnerEvent>;

/// Clients generated bc1, but haven't sent them
pub struct KeygenPhase1Data {
    pub clients: Vec<MultisigClientNoDB>,
    pub bc1_vec: Vec<Broadcast1>,
}

/// Clients generated sec2, but haven't sent them
pub struct KeygenPhase2Data {
    pub clients: Vec<MultisigClientNoDB>,
    /// The key in the map is the index of the desitnation node
    pub sec2_vec: Vec<HashMap<AccountId, Secret2>>,
}

pub struct KeygenPhase3Data {
    pub clients: Vec<MultisigClientNoDB>,
    pub pubkey: secp256k1::PublicKey,
    pub sec_keys: Vec<KeygenResultInfo>,
}

/// Clients received a request to sign and generated Comm1, not broadcast yet
pub struct SigningPhase1Data {
    pub clients: Vec<MultisigClientNoDB>,
    pub comm1_vec: Vec<frost::Comm1>,
}

/// Clients generated Secret2, not sent yet
pub struct SigningPhase2Data {
    pub clients: Vec<MultisigClientNoDB>,
    pub ver2_vec: Vec<frost::VerifyComm2>,
}

pub struct SigningPhase3Data {
    pub clients: Vec<MultisigClientNoDB>,
    pub local_sigs: Vec<frost::LocalSig3>,
}

pub struct SigningPhase4Data {
    pub clients: Vec<MultisigClientNoDB>,
    pub ver4_vec: Vec<frost::VerifyLocalSig4>,
}

pub struct ValidKeygenStates {
    pub keygen_phase1: KeygenPhase1Data,
    pub keygen_phase2: KeygenPhase2Data,
    pub key_ready: KeygenPhase3Data,
}

pub struct ValidSigningStates {
    pub sign_phase1: SigningPhase1Data,
    pub sign_phase2: SigningPhase2Data,
    pub sign_phase3: SigningPhase3Data,
    pub sign_phase4: SigningPhase4Data,
    pub signature: SchnorrSignature,
}

const TEST_PHASE_TIMEOUT: Duration = Duration::from_secs(5);

pub fn keygen_stage_for(client: &MultisigClientNoDB, key_id: KeyId) -> Option<KeygenStage> {
    client.get_keygen().get_stage_for(key_id)
}

pub fn keygen_delayed_count(client: &MultisigClientNoDB, key_id: KeyId) -> usize {
    client.get_keygen().get_delayed_count(key_id)
}

// pub fn signing_delayed_count(client: &MultisigClientNoDB, mi: &MessageInfo) -> usize {
//     client.signing_manager.get_delayed_count(mi)
// }

/// Contains the states at different points of key generation
/// including the final state, where the key is created
pub struct KeygenContext {
    validator_ids: Vec<AccountId>,

    pub rxs: Vec<InnerEventReceiver>,
    /// This clients will match the ones in `key_ready`,
    /// but stored separately so we could substitute
    /// them in more advanced tests
    clients: Vec<MultisigClientNoDB>,
    /// If a test requires a local sig different from
    /// the one that would be normally generated, it
    /// will be stored here
    custom_local_sigs: HashMap<usize, frost::LocalSig3>,
}

impl KeygenContext {
    /// Generate context without starting the
    /// keygen ceremony
    pub fn new() -> Self {
        let validator_ids = (1..=3).map(|idx| AccountId([idx; 32])).collect_vec();
        let logger = logging::test_utils::create_test_logger();
        let (clients, rxs): (Vec<_>, Vec<_>) = validator_ids
            .iter()
            .map(|id| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let c = MultisigClient::new(
                    id.clone(),
                    KeyDBMock::new(),
                    tx,
                    TEST_PHASE_TIMEOUT,
                    &logger,
                );
                (c, rx)
            })
            .unzip();

        KeygenContext {
            validator_ids,
            rxs,
            clients,
            custom_local_sigs: HashMap::new(),
        }
    }

    pub fn get_client(&self, idx: usize) -> &MultisigClientNoDB {
        &self.clients[idx]
    }

    pub fn use_invalid_local_sig(&mut self, signer_idx: usize) {
        use crate::signing::crypto::{ECScalar, FE};

        let sig = frost::LocalSig3 {
            response: FE::new_random(),
        };

        self.custom_local_sigs.insert(signer_idx, sig);
    }

    // Generate keygen states for each of the phases,
    // resulting in `KeygenContext` which can be used
    // to sign messages
    pub async fn generate(&mut self) -> ValidKeygenStates {
        let instant = std::time::Instant::now();

        let clients = &mut self.clients;
        let validator_ids = &self.validator_ids;
        let rxs = &mut self.rxs;

        // Generate phase 1 data

        let key_id = KeyId(0);

        let auction_info = KeygenInfo {
            id: key_id,
            signers: validator_ids.clone(),
        };

        for c in clients.iter_mut() {
            c.process_multisig_instruction(MultisigInstruction::KeyGen(auction_info.clone()));
        }

        let mut bc1_vec = vec![];

        for rx in rxs.iter_mut() {
            let bc1 = recv_bc1_keygen(rx).await;
            bc1_vec.push(bc1);

            // ignore the next message
            let _ = recv_bc1_keygen(rx).await;
        }

        let phase1_clients = clients.clone();

        // *** Distribute BC1, so we can advance and generate Secret2 ***

        for sender_idx in 0..=2 {
            let bc1 = bc1_vec[sender_idx].clone();
            let id = &validator_ids[sender_idx];
            let m = keygen_data_to_p2p(bc1, id, KEY_ID);

            for receiver_idx in 0..=2 {
                if receiver_idx != sender_idx {
                    clients[receiver_idx].process_p2p_mq_message(m.clone());
                }
            }
        }

        for c in clients.iter() {
            assert_eq!(
                keygen_stage_for(c, key_id),
                Some(KeygenStage::AwaitingSecret2)
            );
        }

        let mut sec2_vec = vec![];

        for rx in rxs.iter_mut() {
            let mut sec2_map = HashMap::new();

            // Should generate two messages (one for each of the other two parties)
            for _ in 0u32..2 {
                let (dest, sec2) = recv_secret2_keygen(rx).await;
                sec2_map.insert(dest, sec2);
            }

            sec2_vec.push(sec2_map);
        }

        let phase2_clients = clients.clone();

        let keygen_phase1 = KeygenPhase1Data {
            clients: phase1_clients,
            bc1_vec,
        };

        let keygen_phase2 = KeygenPhase2Data {
            clients: phase2_clients,
            sec2_vec: sec2_vec.clone(),
        };

        // *** Distribute Secret2s, so we can advance and generate Signing Key ***

        for sender_idx in 0..3 {
            for receiver_idx in 0..3 {
                if sender_idx == receiver_idx {
                    continue;
                }

                let r_id = &validator_ids[receiver_idx];
                let sec2 = sec2_vec[sender_idx].get(r_id).unwrap();

                let s_id = &validator_ids[sender_idx];
                let m = keygen_data_to_p2p(sec2.clone(), s_id, KEY_ID);

                clients[receiver_idx].process_p2p_mq_message(m);
            }
        }

        let mut pubkeys = vec![];
        for mut r in rxs.iter_mut() {
            let pubkey = match recv_next_inner_event(&mut r).await {
                InnerEvent::KeygenResult(KeygenOutcome {
                    result: Ok(key), ..
                }) => key,
                _ => panic!("Unexpected inner event"),
            };
            pubkeys.push(pubkey);
        }
        assert_eq!(pubkeys[0].serialize(), pubkeys[1].serialize());
        assert_eq!(pubkeys[1].serialize(), pubkeys[2].serialize());

        let mut sec_keys = vec![];

        for c in clients.iter() {
            let key = c.get_key(KEY_ID).expect("key must be present");
            sec_keys.push(key.clone());
        }

        let keygen_phase3 = KeygenPhase3Data {
            clients: clients.clone(),
            pubkey: pubkeys[0],
            sec_keys,
        };

        println!("Keygen ceremony took: {:?}", instant.elapsed());

        ValidKeygenStates {
            keygen_phase1,
            keygen_phase2,
            key_ready: keygen_phase3,
        }
    }

    pub fn substitute_client_at(
        &mut self,
        idx: usize,
        client: MultisigClientNoDB,
        rx: InnerEventReceiver,
    ) {
        self.clients[idx] = client;
        self.rxs[idx] = rx;
    }

    // Use the generated key and the clients participating
    // in the ceremony and sign a message producing state
    // for each of the signing phases
    pub async fn sign(&mut self) -> ValidSigningStates {
        let instant = std::time::Instant::now();

        let validator_ids = &self.validator_ids;
        let mut clients = self.clients.clone();
        let rxs = &mut self.rxs;

        // *** Send a request to sign and generate BC1 to be distributed ***

        // NOTE: only parties 1 and 2 will participate in signing (SIGNER_IDXS)
        for idx in SIGNER_IDXS.iter() {
            let c = &mut clients[*idx];

            c.process_multisig_instruction(MultisigInstruction::Sign(
                MESSAGE_HASH.clone(),
                SIGN_INFO.clone(),
            ));

            assert_eq!(
                get_stage_for_msg(&c, &MESSAGE_INFO),
                Some("BroadcastStage<AwaitCommitments1>".to_string())
            );
        }

        let mut comm1_vec = vec![];

        for idx in SIGNER_IDXS.iter() {
            let rx = &mut rxs[*idx];

            let comm1 = recv_comm1_signing(rx).await;
            comm1_vec.push(comm1);
        }

        let sign_phase1 = SigningPhase1Data {
            clients: clients.clone(),
            comm1_vec: comm1_vec.clone(),
        };

        assert_channel_empty(&mut rxs[0]).await;

        // *** Broadcast Comm1 messages to advance to Stage2 ***
        for sender_idx in SIGNER_IDXS.iter() {
            let comm1 = comm1_vec[*sender_idx].clone();
            let id = &validator_ids[*sender_idx];

            let m = sig_data_to_p2p(comm1, id, &MESSAGE_INFO);

            for receiver_idx in SIGNER_IDXS.iter() {
                if receiver_idx != sender_idx {
                    clients[*receiver_idx].process_p2p_mq_message(m.clone());
                }
            }
        }

        // TODO: check stage

        // *** Collect Ver2 messages ***

        let mut ver2_vec = vec![];

        for sender_idx in SIGNER_IDXS.iter() {
            let rx = &mut rxs[*sender_idx];

            let ver2 = recv_ver2_signing(rx).await;

            ver2_vec.push(ver2);
        }

        assert_channel_empty(&mut rxs[0]).await;

        let sign_phase2 = SigningPhase2Data {
            clients: clients.clone(),
            ver2_vec: ver2_vec.clone(),
        };

        // *** Distribute Ver2 messages ***

        for sender_idx in SIGNER_IDXS.iter() {
            for receiver_idx in SIGNER_IDXS.iter() {
                if sender_idx != receiver_idx {
                    let ver2 = ver2_vec[*sender_idx].clone();

                    let id = &validator_ids[*sender_idx];
                    let m = sig_data_to_p2p(ver2, id, &MESSAGE_INFO);

                    clients[*receiver_idx].process_p2p_mq_message(m);
                }
            }
        }

        for idx in SIGNER_IDXS.iter() {
            let c = &mut clients[*idx];

            assert_eq!(
                get_stage_for_msg(&c, &MESSAGE_INFO),
                Some("BroadcastStage<LocalSigStage3>".to_string())
            );
        }

        // *** Collect local sigs ***

        let mut local_sigs = vec![];

        for idx in SIGNER_IDXS.iter() {
            let rx = &mut rxs[*idx];

            let sig = recv_local_sig(rx).await;

            // Check if the test requested a custom local sig
            // to be emitted by party idx
            // let sig = self.custom_local_sigs.remove(idx).unwrap_or(sig);
            local_sigs.push(sig);
        }

        assert_channel_empty(&mut rxs[0]).await;

        let sign_phase3 = SigningPhase3Data {
            clients: clients.clone(),
            local_sigs: local_sigs.clone(),
        };

        // *** Distribute local sigs ***

        for sender_idx in SIGNER_IDXS.iter() {
            let local_sig = local_sigs[*sender_idx].clone();
            let id = &validator_ids[*sender_idx];

            let m = sig_data_to_p2p(local_sig, id, &MESSAGE_INFO);

            for receiver_idx in SIGNER_IDXS.iter() {
                if receiver_idx != sender_idx {
                    clients[*receiver_idx].process_p2p_mq_message(m.clone());
                }
            }
        }

        // *** Collect Ver4 messages ***

        let mut ver4_vec = vec![];

        for sender_idx in SIGNER_IDXS.iter() {
            let rx = &mut rxs[*sender_idx];

            let ver4 = recv_ver4_signing(rx).await;

            ver4_vec.push(ver4);
        }

        let sign_phase4 = SigningPhase4Data {
            clients: clients.clone(),
            ver4_vec: ver4_vec.clone(),
        };

        println!("Collected Ver4 messages");

        // *** Distribute Ver4 messages ***

        for sender_idx in SIGNER_IDXS.iter() {
            let ver4 = ver4_vec[*sender_idx].clone();
            let id = &validator_ids[*sender_idx];

            let m = sig_data_to_p2p(ver4, id, &MESSAGE_INFO);

            for receiver_idx in SIGNER_IDXS.iter() {
                if receiver_idx != sender_idx {
                    clients[*receiver_idx].process_p2p_mq_message(m.clone());
                }
            }
        }

        let event = recv_next_inner_event(&mut rxs[0]).await;

        let signature = match event {
            InnerEvent::SigningResult(SigningOutcome {
                result: Ok(sig), ..
            }) => sig,
            _ => panic!("Unexpected event"),
        };

        println!("Signing ceremony took: {:?}", instant.elapsed());

        ValidSigningStates {
            sign_phase1,
            sign_phase2,
            sign_phase3,
            sign_phase4,
            signature,
        }
    }
}

pub async fn assert_channel_empty(rx: &mut InnerEventReceiver) {
    let fut = rx.recv();
    let dur = std::time::Duration::from_millis(10);

    assert!(tokio::time::timeout(dur, fut).await.is_err());
}

/// Skip all non-signal messages
pub async fn recv_next_signal_message_skipping(
    rx: &mut InnerEventReceiver,
) -> Option<SigningOutcome> {
    let dur = std::time::Duration::from_millis(10);

    loop {
        let res = tokio::time::timeout(dur, rx.recv()).await.ok()??;

        if let InnerEvent::SigningResult(s) = res {
            return Some(s);
        }
    }
}

/// Asserts that InnerEvent is in the queue and returns it
pub async fn recv_next_inner_event(rx: &mut InnerEventReceiver) -> InnerEvent {
    let res = check_for_inner_event(rx).await;

    if let Some(event) = res {
        return event;
    }
    panic!("Expected Inner Event");
}

/// checks for an InnerEvent in the que with a short timeout, returns the InnerEvent if there is one.
pub async fn check_for_inner_event(rx: &mut InnerEventReceiver) -> Option<InnerEvent> {
    let dur = std::time::Duration::from_millis(10);
    let res = tokio::time::timeout(dur, rx.recv()).await;
    let opt = res.ok()?;
    opt
}

pub async fn recv_p2p_message(rx: &mut InnerEventReceiver) -> P2PMessageCommand {
    let dur = std::time::Duration::from_millis(10);

    let res = tokio::time::timeout(dur, rx.recv())
        .await
        .ok()
        .expect("timeout")
        .unwrap();

    match res {
        InnerEvent::P2PMessageCommand(m) => m,
        e => {
            eprintln!("Unexpected InnerEvent: {:?}", e);
            panic!();
        }
    }
}

async fn recv_multisig_message(rx: &mut InnerEventReceiver) -> (AccountId, MultisigMessage) {
    let m = recv_p2p_message(rx).await;

    (
        m.destination,
        bincode::deserialize(&m.data).expect("Invalid Multisig Message"),
    )
}

async fn recv_bc1_keygen(rx: &mut InnerEventReceiver) -> Broadcast1 {
    let (_, m) = recv_multisig_message(rx).await;

    if let MultisigMessage::KeyGenMessage(wrapped) = m {
        let KeyGenMessageWrapped { message, .. } = wrapped;

        if let KeygenData::Broadcast1(bc1) = message {
            return bc1;
        }
    }

    eprintln!("Received message is not Broadcast1 (keygen)");
    panic!();
}

async fn recv_comm1_signing(rx: &mut InnerEventReceiver) -> frost::Comm1 {
    let (_, m) = recv_multisig_message(rx).await;

    if let MultisigMessage::SigningMessage(SigningDataWrapped { data, .. }) = m {
        if let SigningData::CommStage1(comm1) = data {
            return comm1;
        }
    }

    eprintln!("Received message is not Comm1 (signing)");
    panic!();
}

async fn recv_local_sig(rx: &mut InnerEventReceiver) -> frost::LocalSig3 {
    let (_, m) = recv_multisig_message(rx).await;

    if let MultisigMessage::SigningMessage(SigningDataWrapped { data, .. }) = m {
        if let SigningData::LocalSigStage3(sig) = data {
            return sig;
        }
    }

    eprintln!("Received message is not LocalSig");
    panic!();
}

async fn recv_secret2_keygen(rx: &mut InnerEventReceiver) -> (AccountId, Secret2) {
    let (dest, m) = recv_multisig_message(rx).await;

    if let MultisigMessage::KeyGenMessage(wrapped) = m {
        let KeyGenMessageWrapped { message, .. } = wrapped;

        if let KeygenData::Secret2(sec2) = message {
            return (dest, sec2);
        }
    }

    eprintln!("Received message is not Secret2 (keygen)");
    panic!();
}

async fn recv_ver2_signing(rx: &mut InnerEventReceiver) -> frost::VerifyComm2 {
    let (_, m) = recv_multisig_message(rx).await;

    if let MultisigMessage::SigningMessage(SigningDataWrapped { data, .. }) = m {
        if let SigningData::BroadcastVerificationStage2(ver2) = data {
            return ver2;
        }
    }

    eprintln!("Received message is not Secret2 (signing)");
    panic!();
}

async fn recv_ver4_signing(rx: &mut InnerEventReceiver) -> frost::VerifyLocalSig4 {
    let (_, m) = recv_multisig_message(rx).await;

    if let MultisigMessage::SigningMessage(SigningDataWrapped { data, .. }) = m {
        if let SigningData::VerifyLocalSigsStage4(ver4) = data {
            return ver4;
        }
    }

    eprintln!("Received message is not Secret2 (signing)");
    panic!();
}

pub fn sig_data_to_p2p(
    data: impl Into<SigningData>,
    sender_id: &AccountId,
    mi: &MessageInfo,
) -> P2PMessage {
    let wrapped = SigningDataWrapped::new(data, mi.clone());

    let data = MultisigMessage::from(wrapped);
    let data = bincode::serialize(&data).unwrap();
    P2PMessage {
        sender_id: sender_id.clone(),
        data,
    }
}

pub fn keygen_data_to_p2p(
    data: impl Into<KeygenData>,
    sender_id: &AccountId,
    key_id: KeyId,
) -> P2PMessage {
    let wrapped = KeyGenMessageWrapped::new(key_id, data);

    let data = MultisigMessage::from(wrapped);
    let data = bincode::serialize(&data).unwrap();

    P2PMessage {
        sender_id: sender_id.clone(),
        data,
    }
}

pub fn get_stage_for_msg(c: &MultisigClientNoDB, message_info: &MessageInfo) -> Option<String> {
    c.signing_manager.get_stage_for(message_info)
}

pub fn create_bc1(signer_idx: usize) -> Broadcast1 {
    let key = Keys::phase1_create(signer_idx);

    let (bc1, blind) = key.phase1_broadcast();

    let y_i = key.y_i;

    Broadcast1 { bc1, blind, y_i }
}

pub fn create_invalid_bc1() -> Broadcast1 {
    let key = Keys::phase1_create(0);

    let key2 = Keys::phase1_create(0);

    let (_, blind) = key.phase1_broadcast();

    let (bc1, _) = key2.phase1_broadcast();

    let y_i = key.y_i;

    Broadcast1 { bc1, blind, y_i }
}
