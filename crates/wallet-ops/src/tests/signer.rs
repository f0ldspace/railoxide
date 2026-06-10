use super::helpers::*;

#[test]
fn software_evm_signer_uses_separate_transaction_and_message_boundaries() {
    fn exercise_boundaries(signer: &(impl EvmTransactionSigner + EvmMessageSigner)) {
        let address = signer.address();
        let shield_key = signer
            .derive_shield_private_key()
            .expect("derive shield key through EVM message boundary");
        let _wallet = signer.ethereum_wallet();

        assert_ne!(address, Address::ZERO);
        assert_ne!(shield_key, [0u8; 32]);
    }

    let signer = SoftwareEvmSigner::from_private_key([1; 32]).expect("software EVM signer");

    exercise_boundaries(&signer);
}
