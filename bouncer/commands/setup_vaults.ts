#!/usr/bin/env -S pnpm tsx
// INSTRUCTIONS
//
// This command takes no arguments.
// It will perform the initial polkadot vault setup procedure described here
// https://www.notion.so/chainflip/Polkadot-Vault-Initialisation-Steps-36d6ab1a24ed4343b91f58deed547559
// For example: ./commands/setup_vaults.ts

import { AddressOrPair } from '@polkadot/api/types';
import Web3 from 'web3';
import { submitGovernanceExtrinsic } from '../shared/cf_governance';
import {
  getPolkadotApi,
  getBtcClient,
  handleSubstrateError,
  getEvmEndpoint,
  getSolConnection,
  deferredPromise,
} from '../shared/utils';
import { aliceKeyringPair } from '../shared/polkadot_keyring';
import {
  initializeArbitrumChain,
  initializeArbitrumContracts,
  initializeSolanaPrograms,
} from '../shared/initialize_new_chains';
import { observeEvent } from '../shared/utils/substrate';

async function main(): Promise<void> {
  const btcClient = getBtcClient();
  const arbClient = new Web3(getEvmEndpoint('Arbitrum'));
  const alice = await aliceKeyringPair();
  const solClient = getSolConnection();

  await using polkadot = await getPolkadotApi();

  console.log('=== Performing initial Vault setup ===');

  // Step 1
  await initializeArbitrumChain();

  // Step 2
  console.log('Forcing rotation');
  await submitGovernanceExtrinsic((api) => api.tx.validator.forceRotation());

  // Step 3
  console.log('Waiting for new keys');

  const dotActivationRequest = observeEvent('polkadotVault:AwaitingGovernanceActivation').event;
  const btcActivationRequest = observeEvent('bitcoinVault:AwaitingGovernanceActivation').event;
  const arbActivationRequest = observeEvent('arbitrumVault:AwaitingGovernanceActivation').event;
  // const solActivationRequest = observeEvent('solanaVault:AwaitingGovernanceActivation', chainflip);
  const dotKey = (await dotActivationRequest).data.newPublicKey;
  const btcKey = (await btcActivationRequest).data.newPublicKey;
  const arbKey = (await arbActivationRequest).data.newPublicKey;
  // const solKey = (await solActivationRequest).data.newPublicKey;

  // Step 4
  console.log('Requesting Polkadot Vault creation');
  const createPolkadotVault = async () => {
    const { promise, resolve } = deferredPromise<{
      vaultAddress: AddressOrPair;
      vaultExtrinsicIndex: number;
    }>();

    const unsubscribe = await polkadot.tx.proxy
      .createPure(polkadot.createType('ProxyType', 'Any'), 0, 0)
      .signAndSend(alice, { nonce: -1 }, (result) => {
        if (result.isError) {
          handleSubstrateError(result);
        }
        if (result.isInBlock) {
          console.log('Polkadot Vault created');
          // TODO: figure out type inference so we don't have to coerce using `any`
          const pureCreated = result.findRecord('proxy', 'PureCreated')!;
          resolve({
            vaultAddress: pureCreated.event.data[0] as AddressOrPair,
            vaultExtrinsicIndex: result.txIndex!,
          });
          unsubscribe();
        }
      });

    return promise;
  };
  const { vaultAddress, vaultExtrinsicIndex } = await createPolkadotVault();

  const proxyAdded = observeEvent('proxy:ProxyAdded', { chain: 'polkadot' }).event;

  // Step 5
  console.log('Rotating Proxy and Funding Accounts.');
  const rotateAndFund = async () => {
    const { promise, resolve } = deferredPromise<void>();
    const rotation = polkadot.tx.proxy.proxy(
      polkadot.createType('MultiAddress', vaultAddress),
      null,
      polkadot.tx.utility.batchAll([
        polkadot.tx.proxy.addProxy(
          polkadot.createType('MultiAddress', dotKey),
          polkadot.createType('ProxyType', 'Any'),
          0,
        ),
        polkadot.tx.proxy.removeProxy(
          polkadot.createType('MultiAddress', alice.address),
          polkadot.createType('ProxyType', 'Any'),
          0,
        ),
      ]),
    );

    const unsubscribe = await polkadot.tx.utility
      .batchAll([
        // Note the vault needs to be funded before we rotate.
        polkadot.tx.balances.transfer(vaultAddress, 1000000000000),
        polkadot.tx.balances.transfer(dotKey, 1000000000000),
        rotation,
      ])
      .signAndSend(alice, { nonce: -1 }, (result) => {
        if (result.isError) {
          handleSubstrateError(result);
        }
        if (result.isInBlock) {
          unsubscribe();
          resolve();
        }
      });

    await promise;
  };
  await rotateAndFund();
  const vaultBlockNumber = (await proxyAdded).block;

  // Step 6
  console.log('Inserting Arbitrum key in the contracts');
  await initializeArbitrumContracts(arbClient, arbKey);

  // Using arbitrary key for now, we will use solKey generated by SC
  const solKey = '0x25fcb03ab6435d106b5df1e677f3c6a10a7b22719deedeb3761c005e1306423d';
  await initializeSolanaPrograms(solClient, solKey);

  // Step 7
  console.log('Registering Vaults with state chain');
  await submitGovernanceExtrinsic((chainflip) =>
    chainflip.tx.environment.witnessPolkadotVaultCreation(vaultAddress, {
      blockNumber: vaultBlockNumber,
      extrinsicIndex: vaultExtrinsicIndex,
    }),
  );
  await submitGovernanceExtrinsic(async (chainflip) =>
    chainflip.tx.environment.witnessCurrentBitcoinBlockNumberForKey(
      await btcClient.getBlockCount(),
      btcKey,
    ),
  );

  await submitGovernanceExtrinsic(async (chainflip) =>
    chainflip.tx.environment.witnessInitializeArbitrumVault(await arbClient.eth.getBlockNumber()),
  );

  // TODO: We can insert program ID, nonces accounts, durable nonces, vaultPda and its seed/bump,
  // tokenVault and its seed/bump, data account ID and its seed/bump, in the runtime upgrade.
  // Only issue reg. durable nonces is that they will need to be changed every time a new Solana tag is
  // used since they are not deterministic. We could insert them in the governance extrinsic but that
  // is unnecessary for production.
  // await submitGovernanceExtrinsic(chainflip.tx.environment.witnessInitializeSolanaVault());

  // Confirmation
  console.log('Waiting for new epoch...');
  await observeEvent('validator:NewEpoch');

  console.log('=== New Epoch ===');
  console.log('=== Vault Setup completed ===');
  process.exit(0);
}

main().catch((error) => {
  console.error(error);
  process.exit(-1);
});
