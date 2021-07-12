use std::{collections::HashMap, convert::TryInto};

use crate::{
    eth::key_manager::KeyManager,
    mq::{pin_message_stream, IMQClient, Subject},
    p2p::ValidatorId,
    settings,
    signing::{
        crypto::Signature, KeyId, KeygenOutcome, KeygenSuccess, MessageHash, MessageInfo,
        MultisigEvent, MultisigInstruction, SigningInfo, SigningOutcome, SigningSuccess,
    },
    types::chain::Chain,
};

use anyhow::Result;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sp_core::Hasher;
use sp_runtime::traits::Keccak256;
use web3::{ethabi::Token, types::Address};

use curv::{
    arithmetic::Converter,
    elliptic::curves::{
        secp256_k1::Secp256k1Point,
        traits::{ECPoint, ECScalar},
    },
};
use secp256k1::PublicKey;

/// Helper function, constructs and runs the [SetAggKeyWithAggKeyEncoder] asynchronously.
pub async fn start<M: IMQClient + Clone>(
    settings: &settings::Settings,
    mq_client: M,
) -> Result<()> {
    let mut encoder = SetAggKeyWithAggKeyEncoder::new(
        settings.eth.key_manager_eth_address.as_ref(),
        settings.signing.genesis_validator_ids.clone(),
        mq_client,
    )?;

    encoder.process_multi_sig_event_stream().await;

    Ok(())
}

/// Details of a transaction to be broadcast to ethereum.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TxDetails {
    pub contract_address: Address,
    pub data: Vec<u8>,
}

/// Reads [AuctionConfirmedEvent]s off the message queue and encodes the function call to the stake manager.
#[derive(Clone)]
struct SetAggKeyWithAggKeyEncoder<M: IMQClient> {
    mq_client: M,
    key_manager: KeyManager,
    // maps the MessageHash which gets sent to the signer with the data that the MessageHash is a hash of
    messages: HashMap<MessageHash, ParamContainer>,
    // On genesis, where do these validators come from, to allow for the first key update
    validators: HashMap<KeyId, Vec<ValidatorId>>,
    curr_signing_key_id: Option<KeyId>,
    next_key_id: Option<KeyId>,
}

#[derive(Clone)]
struct ParamContainer {
    pub key_id: KeyId,
    pub key_nonce: [u8; 32],
    pub pubkey_x: [u8; 32],
    pub pubkey_y_parity: u8,
}

impl<M: IMQClient + Clone> SetAggKeyWithAggKeyEncoder<M> {
    fn new(
        key_manager_address: &str,
        genesis_validator_ids: Vec<ValidatorId>,
        mq_client: M,
    ) -> Result<Self> {
        let key_manager = KeyManager::load(key_manager_address)?;

        let mut genesis_validator_ids_hash_map = HashMap::new();
        genesis_validator_ids_hash_map.insert(KeyId(0), genesis_validator_ids);
        Ok(Self {
            mq_client,
            key_manager,
            messages: HashMap::new(),
            validators: genesis_validator_ids_hash_map,
            curr_signing_key_id: Some(KeyId(0)),
            next_key_id: None,
        })
    }

    /// Read events from the MultisigEvent subject and process them
    /// The messages we care about are:
    /// 1. `MultisigEvent::KeygenResult` which is emitted after a new key has been
    /// successfully generated by the signing module
    /// 2. `MultisigEvent::MessagedSigned` which is emitted after the Signing module
    /// has successfully signed a message with a particular (denoted by KeyId) key
    async fn process_multi_sig_event_stream(&mut self) {
        let multisig_event_stream = self
            .mq_client
            .subscribe::<MultisigEvent>(Subject::MultisigEvent)
            .await
            .unwrap();

        let mut multisig_event_stream = pin_message_stream(multisig_event_stream);

        while let Some(event) = multisig_event_stream.next().await {
            match event {
                Ok(event) => match event {
                    MultisigEvent::KeygenResult(key_outcome) => match key_outcome {
                        KeygenOutcome::Success(keygen_success) => {
                            self.handle_keygen_success(keygen_success).await;
                        }
                        _ => {
                            log::error!("Signing module returned error generating key")
                        }
                    },
                    MultisigEvent::MessageSigningResult(signing_outcome) => match signing_outcome {
                        SigningOutcome::MessageSigned(signing_success) => {
                            self.handle_set_agg_key_message_signed(signing_success)
                                .await;
                        }
                        _ => {
                            // TODO: Use the reported bad nodes in the SigningOutcome / SigningFailure
                            // TODO: retry signing with a different subset of signers
                            log::error!("Signing module returned error signing message")
                        }
                    },
                    _ => {
                        log::trace!("Discarding non keygen result or message signed event")
                    }
                },
                Err(e) => {
                    log::error!("Error reading event from multisig event stream: {:?}", e);
                }
            }
        }
    }

    // When the keygen message has been received we must:
    // 1. Build the ETH encoded setAggKeyWithAggKey transaction parameters
    // 2. Store the tx parameters in state for use later
    // 3. Create a Signing Instruction
    // 4. Push this instruction to the MQ for the signing module to pick up
    async fn handle_keygen_success(&mut self, keygen_success: KeygenSuccess) {
        let (encoded_fn_params, param_container) = self
            .build_encoded_fn_params(&keygen_success)
            .expect("should be a valid encoded params");

        let hash = Keccak256::hash(&encoded_fn_params[..]);
        let message_hash = MessageHash(hash.0);

        // store key: parameters, so we can fetch the parameters again, after the payload
        // has been signed by the signing module
        self.messages.insert(message_hash.clone(), param_container);

        // Use *all* the validators for now
        let key_id = self.curr_signing_key_id.expect("KeyId should be set here");
        let signing_info = SigningInfo::new(
            key_id,
            self.validators
                .get(&key_id)
                .expect("validators should exist for current KeyId")
                .clone(),
        );

        let signing_instruction = MultisigInstruction::Sign(message_hash, signing_info);

        self.mq_client
            .publish(Subject::MultisigInstruction, &signing_instruction)
            .await
            .expect("Should publish to MQ");
    }

    fn point_to_pubkey(&self, point: Secp256k1Point) -> PublicKey {
        let bytes_x: [u8; 32] = point
            .x_coor()
            .expect("Valid point should have an x coordinate")
            .to_bytes()
            .try_into()
            .expect("Valid point x_coor should contain only 32 bytes");

        let bytes_x = &mut bytes_x.to_vec();

        let bytes_y: [u8; 32] = point
            .y_coor()
            .expect("valid point should have a y coordinate")
            .to_bytes()
            .try_into()
            .expect("should be 32 bytes");

        let bytes_y = &mut bytes_y.to_vec();

        let mut bytes = Vec::new();
        bytes.push(0x04);
        bytes.append(bytes_x);
        bytes.append(bytes_y);

        let pubkey = PublicKey::from_slice(&bytes);

        return pubkey.expect("Should be valid pubkey");
    }

    // When the signed message has been received we must:
    // 1. Get the parameters (`ParameterContainer`) that we stored in state (and submitted to the signing module in encoded form) earlier
    // 2. Build a valid ethereum encoded transaction using the message hash and signature returned by the Signing module
    // 3. Push this transaction to the Broadcast(Chain::ETH) subject, to be broadcast by the ETH Broadcaster
    // 4. Update the current key id, with the new key id returned by the signing module, so we know which key to sign with
    // from now onwards, until the next successful key rotation
    async fn handle_set_agg_key_message_signed(&mut self, signing_success: SigningSuccess) {
        // 1. Get the data from the message hash that was signed (using the `messages` field)
        let sig = signing_success.sig;
        let message_info = signing_success.message_info;
        let k_g = self.point_to_pubkey(sig.v);
        let nonce_times_g_addr = self.nonce_times_g_addr_from_v(k_g);

        let key_id = message_info.key_id;
        let msg_hash = message_info.hash;
        let params = self
            .messages
            .get(&msg_hash)
            .expect("should have been stored when asked to sign");

        // 2. Call build_tx with the required info
        match self.build_tx(&msg_hash, &sig, nonce_times_g_addr, params) {
            Ok(ref tx_details) => {
                // 3. Send it on its way to the eth broadcaster
                self.mq_client
                    .publish(Subject::Broadcast(Chain::ETH), tx_details)
                    .await
                    .unwrap_or_else(|err| {
                        log::error!("Could not process: {:#?}", err);
                    });
                // here (for now) we assume the key was update successfully
                // update curr key id
                self.curr_signing_key_id = Some(key_id);
                // reset
                self.next_key_id = None;
            }
            Err(err) => {
                log::error!("Failed to build: {:#?}", err);
            }
        }
    }

    fn build_tx(
        &self,
        msg: &MessageHash,
        sig: &Signature,
        nonce_times_g_addr: [u8; 20],
        params: &ParamContainer,
    ) -> Result<TxDetails> {
        let s: [u8; 32] = sig
            .sigma
            .to_big_int()
            .to_bytes()
            .try_into()
            .expect("Should be a valid Signature scalar");
        let params = self.set_agg_key_with_agg_key_param_constructor(
            msg.0,
            s,
            params.key_nonce,
            nonce_times_g_addr,
            params.pubkey_x,
            params.pubkey_y_parity,
        );

        let tx_data = self.encode_params_key_manager_fn(params)?;

        Ok(TxDetails {
            contract_address: self.key_manager.deployed_address,
            data: tx_data.into(),
        })
    }

    /// v is 'r' in the literature. r = k * G where k is the nonce and G is the address generator
    /// i.e. k * G is the pubkey generated from secret key "k"
    fn nonce_times_g_addr_from_v(&self, v: secp256k1::PublicKey) -> [u8; 20] {
        let v_pub: [u8; 64] = v.serialize_uncompressed()[1..]
            .try_into()
            .expect("Should be a valid pubkey");

        // calculate nonce times g addr - the hash over
        let nonce_times_g_addr_hash = Keccak256::hash(&v_pub).as_bytes().to_owned();

        // take the last 160bits (20 bytes)
        let nonce_times_g_addr: [u8; 20] = nonce_times_g_addr_hash[140..]
            .try_into()
            .expect("Should only be 20 bytes long");

        return nonce_times_g_addr;
    }

    // Take a secp256k1 pubkey and return the pubkey_x and pubkey_y_parity
    fn destructure_pubkey(&self, pubkey: secp256k1::PublicKey) -> ([u8; 32], u8) {
        let pubkey_bytes: [u8; 33] = pubkey.serialize();
        let pubkey_y_parity_byte = pubkey_bytes[0];
        let pubkey_y_parity = if pubkey_y_parity_byte == 2 { 0u8 } else { 1u8 };
        let pubkey_x: [u8; 32] = pubkey_bytes[1..].try_into().expect("Is valid pubkey");

        return (pubkey_x, pubkey_y_parity);
    }

    // This has nothing to do with building an ETH transaction.
    // We encode the tx like this, in eth format, because this is how the contract will
    // serialise the data to verify the signature over the message hash
    fn build_encoded_fn_params(
        &self,
        keygen_success: &KeygenSuccess,
    ) -> Result<(Vec<u8>, ParamContainer)> {
        let pubkey = keygen_success.key;

        let (pubkey_x, pubkey_y_parity) = self.destructure_pubkey(pubkey);

        let param_container = ParamContainer {
            key_id: keygen_success.key_id,
            pubkey_x,
            pubkey_y_parity,
            key_nonce: [0u8; 32],
        };

        let params = self.set_agg_key_with_agg_key_param_constructor(
            [0u8; 32],
            [0u8; 32],
            [0u8; 32],
            [0u8; 20],
            pubkey_x,
            pubkey_y_parity,
        );

        let encoded_data = self.encode_params_key_manager_fn(params)?;

        return Ok((encoded_data, param_container));
    }

    fn encode_params_key_manager_fn(&self, params: [Token; 2]) -> Result<Vec<u8>> {
        // Serialize the data using eth encoding so the KeyManager contract can serialize the data in the same way
        // in order to verify the signature
        let encoded_data = self
            .key_manager
            .set_agg_key_with_agg_key()
            .encode_input(&params[..])?;

        return Ok(encoded_data);
    }

    // not sure if key nonce should be u64...
    // sig = s in the literature. The scalar of the signature
    fn set_agg_key_with_agg_key_param_constructor(
        &self,
        msg_hash: [u8; 32],
        sig: [u8; 32],
        key_nonce: [u8; 32],
        nonce_times_g_addr: [u8; 20],
        pubkey_x: [u8; 32],
        pubkey_y_parity: u8,
    ) -> [Token; 2] {
        // These are two arguments, SigData and Key from:
        // https://github.com/chainflip-io/chainflip-eth-contracts/blob/master/contracts/interfaces/IShared.sol
        [
            // SigData
            Token::Tuple(vec![
                Token::Uint(msg_hash.into()),              // msgHash
                Token::Uint(sig.into()), // sig - this 's' in the literature, the signature scalar
                Token::Uint(key_nonce.into()), // key nonce
                Token::Address(nonce_times_g_addr.into()), // nonceTimesGAddr - this is r in the literature
            ]),
            // Key - the signing module will sign over the params, containing this
            Token::Tuple(vec![
                Token::Uint(pubkey_x.into()),        // pubkeyX
                Token::Uint(pubkey_y_parity.into()), // pubkeyYparity
            ]),
        ]
    }
}

#[cfg(test)]
mod test_eth_tx_encoder {
    use super::*;
    use curv::arithmetic::Converter;
    use hex;
    use num::BigInt;
    use std::str::FromStr;

    use crate::mq::mq_mock::MQMock;

    #[test]
    fn test_point_to_pubkey() {
        // Serialized point: "{\"x\":\"8d13221e3a7326a34dd45214ba80116dd142e4b5ff3ce66a8dc7bfa0378b795\",\"y\":\"5d41ac1477614b5c0848d50dbd565ea2807bcba1df0df07a8217e9f7f7c2be88\"}"
        const BASE_POINT2_X: [u8; 32] = [
            0x08, 0xd1, 0x32, 0x21, 0xe3, 0xa7, 0x32, 0x6a, 0x34, 0xdd, 0x45, 0x21, 0x4b, 0xa8,
            0x01, 0x16, 0xdd, 0x14, 0x2e, 0x4b, 0x5f, 0xf3, 0xce, 0x66, 0xa8, 0xdc, 0x7b, 0xfa,
            0x03, 0x78, 0xb7, 0x95,
        ];
        const BASE_POINT2_Y: [u8; 32] = [
            0x5d, 0x41, 0xac, 0x14, 0x77, 0x61, 0x4b, 0x5c, 0x08, 0x48, 0xd5, 0x0d, 0xbd, 0x56,
            0x5e, 0xa2, 0x80, 0x7b, 0xcb, 0xa1, 0xdf, 0x0d, 0xf0, 0x7a, 0x82, 0x17, 0xe9, 0xf7,
            0xf7, 0xc2, 0xbe, 0x88,
        ];

        let big_int_x = curv::BigInt::from_bytes(&BASE_POINT2_X);
        let big_int_y = curv::BigInt::from_bytes(&BASE_POINT2_Y);
        let point: Secp256k1Point = Secp256k1Point::from_coor(&big_int_x, &big_int_y);

        // assert on point to pubkey
        let mq = MQMock::new();

        let mq_client = mq.get_client();

        let settings = settings::test_utils::new_test_settings().unwrap();

        let encoder = SetAggKeyWithAggKeyEncoder::new(
            settings.eth.key_manager_eth_address.as_str(),
            settings.signing.genesis_validator_ids,
            mq_client,
        )
        .unwrap();

        // we rotate to key 2, so this is the pubkey we want to sign over
        // TODO: Set the actual expected pubkey. THis is not it, it's a random pubkey
        // (and I'd rather it not be generated from my own implementation)
        let expected_pubkey = PublicKey::from_str(
            "0218845781f631c48f1c9709e23092067d06837f30aa0cd0544ac887fe91ddd166",
        )
        .unwrap();

        let pubkey = encoder.point_to_pubkey(point);
        println!("expected pubkey: {}", expected_pubkey);
        println!("got pubkey: {}", pubkey);
        // TODO: Add the assert over pubkey and expected
    }

    #[test]
    fn test_message_hashing() {
        // The data is generated from: https://github.com/chainflip-io/chainflip-eth-contracts/blob/master/tests/integration/keyManager/test_setKey_setKey.py

        // sig data from contract, we aren't testing signing, so we use these values, generated from the contract tests
        // messageHashHex as an int int("{messageHashHex}", 16)), s / sig scalar, key nonce, nonce_times_g_addr
        // [19838331578708755702960229198816480402256567085479269042839672688267843389518, 86256580123538456061655860770396085945007591306530617821168588559087896188216, 0, '02eDd8421D87B7c0eE433D3AFAd3aa2Ef039f27a']

        // params used:
        // AGG_PRIV_HEX_1 = "fbcb47bc85b881e0dfb31c872d4e06848f80530ccbd18fc016a27c4a744d0eba"
        // AGG_K_HEX_1 = "d51e13c68bf56155a83e50fd9bc840e2a1847fb9b49cd206a577ecd1cd15e285"
        // AGG_SIGNER_1 = Signer(AGG_PRIV_HEX_1, AGG_K_HEX_1, AGG, nonces)
        // JUNK_HEX_PAD = 0000000000000000000000000000000000000000000000000000000000003039

        // We move to these keys
        // AGG_PRIV_HEX_2 = "bbade2da39cfc81b1b64b6a2d66531ed74dd01803dc5b376ce7ad548bbe23608"
        // AGG_K_HEX_2 = "ecb77b2eb59614237e5646b38bdf03cbdbdce61c874fdee6e228edaa26f01f5d"
        // AGG_SIGNER_2 = Signer(AGG_PRIV_HEX_2, AGG_K_HEX_2, AGG, nonces)

        // Pub data
        // [22479114112312168431982914496826057754130808976066989807481484372215659188398, 1]

        let mq = MQMock::new();

        let mq_client = mq.get_client();

        let settings = settings::test_utils::new_test_settings().unwrap();

        let encoder = SetAggKeyWithAggKeyEncoder::new(
            settings.eth.key_manager_eth_address.as_str(),
            settings.signing.genesis_validator_ids,
            mq_client,
        )
        .unwrap();

        let s = secp256k1::Secp256k1::signing_only();
        let sk_1 = secp256k1::SecretKey::from_str(
            "fbcb47bc85b881e0dfb31c872d4e06848f80530ccbd18fc016a27c4a744d0eba",
        )
        .unwrap();

        let sk_2 = secp256k1::SecretKey::from_str(
            "bbade2da39cfc81b1b64b6a2d66531ed74dd01803dc5b376ce7ad548bbe23608",
        )
        .unwrap();

        // we rotate to key 2, so this is the pubkey we want to sign over
        let pubkey_from_sk_2 = PublicKey::from_secret_key(&s, &sk_2);

        let (pubkey_x, pubkey_y_parity) = encoder.destructure_pubkey(pubkey_from_sk_2);

        // hash_junk_bytes.try_into().unwrap(),
        let params = encoder.set_agg_key_with_agg_key_param_constructor(
            [0u8; 32],
            [0u8; 32],
            [0u8; 32],
            [0u8; 20],
            pubkey_x,
            pubkey_y_parity,
        );

        let encoded = encoder.encode_params_key_manager_fn(params).unwrap();
        let hex_params = hex::encode(&encoded);
        println!("hex params: {:#?}", hex_params);
        // hex - from smart contract tests
        let call_data_no_sig_from_contract = "24969d5d00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001742daacd4dbfbe66d4c8965550295873c683cb3b65019d3a53975ba553cc31d0000000000000000000000000000000000000000000000000000000000000001";
        assert_eq!(call_data_no_sig_from_contract, hex_params);

        let message_hash: [u8; 32] = BigInt::from_str(
            "19838331578708755702960229198816480402256567085479269042839672688267843389518",
        )
        .unwrap()
        .to_bytes_be()
        .1
        .try_into()
        .unwrap();

        let message_hash = MessageHash(message_hash);

        let sig: num::BigInt = BigInt::from_str(
            "86256580123538456061655860770396085945007591306530617821168588559087896188216",
        )
        .unwrap();

        let param_container = ParamContainer {
            key_id: KeyId(0),
            key_nonce: [0u8; 32],
            pubkey_x,
            pubkey_y_parity,
        };

        let nonce_times_g_addr = hex::decode("02eDd8421D87B7c0eE433D3AFAd3aa2Ef039f27a")
            .unwrap()
            .try_into()
            .unwrap();

        // to big endian bytes then into the curv type
        let curv_sig_big_int = curv::BigInt::from_bytes(&sig.to_bytes_be().1);

        // TODO: Create some utils to clean this up
        // v = r = k * G. In the contract we use:
        let k = "d51e13c68bf56155a83e50fd9bc840e2a1847fb9b49cd206a577ecd1cd15e285";
        let k_as_sk = secp256k1::SecretKey::from_str(k).unwrap();
        let k_times_g = PublicKey::from_secret_key(&s, &k_as_sk);
        let k_times_g_bytes: [u8; 32] = k_times_g.serialize()[1..].try_into().unwrap();

        // this is the struct of the Signature returned from the signing module
        let sig = Signature {
            sigma: curv::elliptic::curves::traits::ECScalar::from(&curv_sig_big_int),
            v: Secp256k1Point::from_bytes(&k_times_g_bytes).unwrap(),
        };

        let tx_data = encoder
            .build_tx(&message_hash, &sig, nonce_times_g_addr, &param_container)
            .unwrap()
            .data;

        let eth_input_from_receipt = "24969d5d2bdc19071c7994f088103dbf8d5476d6deb6d55ee005a2f510dc7640055cc84ebeb37e87509e15cd88b19fa224441c56acc0e143cb25b9fd1e57fdafed215538000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002edd8421d87b7c0ee433d3afad3aa2ef039f27a1742daacd4dbfbe66d4c8965550295873c683cb3b65019d3a53975ba553cc31d0000000000000000000000000000000000000000000000000000000000000001";

        assert_eq!(eth_input_from_receipt.to_string(), hex::encode(&tx_data));
    }

    #[test]
    fn secp256k1_sanity_check() {
        let s = secp256k1::Secp256k1::signing_only();

        let sk = secp256k1::SecretKey::from_str(
            "fbcb47bc85b881e0dfb31c872d4e06848f80530ccbd18fc016a27c4a744d0eba",
        )
        .unwrap();

        let pubkey_from_sk = PublicKey::from_secret_key(&s, &sk);

        // these keys should be derivable from each other.
        let pubkey = secp256k1::PublicKey::from_str(
            "0331b2ba4b46201610901c5164f42edd1f64ce88076fde2e2c544f9dc3d7b350ae",
        )
        .unwrap();

        // for sanity
        assert_eq!(pubkey_from_sk, pubkey);
    }
}
