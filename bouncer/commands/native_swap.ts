import { executeSwap } from '@chainflip-io/cli';
import { u8aToHex } from '@polkadot/util';
import { decodeAddress } from '@polkadot/util-crypto';
import { Wallet, getDefaultProvider } from 'ethers';
import { chainFromToken } from '../shared/utils';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function executeNativeSwap(destTokenSymbol: any, destinationAddress: string) {

    const wallet = Wallet.fromMnemonic(
        'test test test test test test test test test test test junk',
    ).connect(getDefaultProvider('http://localhost:8545'));

    const destToken = destTokenSymbol.toUpperCase();
    const destChainId = chainFromToken(destToken);

    let destAddress = destinationAddress;

    if (destToken === 'DOT') {
        destAddress = u8aToHex(decodeAddress(destAddress))
    }

    await executeSwap(
        {
            destChainId,
            destTokenSymbol: destToken,
            // It is important that this is large enough to result in
            // an amount larger than existential (e.g. on Polkadot):
            amount: '100000000000000000',
            destAddress,
        },
        {
            signer: wallet,
            cfNetwork: 'localnet',
            vaultContractAddress: '0xb7a5bd0345ef1cc5e66bf61bdec17d2461fbd968',
        },
    );
}