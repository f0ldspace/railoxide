use crate::walletconnect::WalletConnectError;
use crate::walletconnect::crypto::{
    WalletConnectEnvelope, decode_walletconnect_message, derive_walletconnect_session_sym_key,
    derive_walletconnect_session_topic, encode_walletconnect_message_with_nonce,
    hash_walletconnect_key,
};

#[test]
fn encrypted_envelope_round_trips_and_decodes_from_base64() {
    let sym_key = [1u8; 32];
    let nonce = [2u8; 12];
    let envelope =
        encode_walletconnect_message_with_nonce(&sym_key, br#"{"id":1}"#, nonce).unwrap();
    let encoded = envelope.to_base64();
    let decoded = WalletConnectEnvelope::from_base64(&encoded).unwrap();
    let plaintext = decode_walletconnect_message(&sym_key, &decoded).unwrap();

    assert_eq!(plaintext, br#"{"id":1}"#);
}

#[test]
fn malformed_envelope_or_wrong_key_is_rejected() {
    assert!(WalletConnectEnvelope::from_base64("not base64").is_err());

    let envelope =
        encode_walletconnect_message_with_nonce(&[1u8; 32], b"payload", [2u8; 12]).unwrap();
    assert!(decode_walletconnect_message(&[3u8; 32], &envelope).is_err());
}

#[test]
fn session_topic_is_hash_of_derived_symmetric_key() {
    let private_a = [1u8; 32];
    let public_a = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(private_a));
    let private_b = [2u8; 32];
    let public_b = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(private_b));

    let sym_key_a = derive_walletconnect_session_sym_key(&private_a, public_b.as_bytes()).unwrap();
    let sym_key_b = derive_walletconnect_session_sym_key(&private_b, public_a.as_bytes()).unwrap();

    assert_eq!(sym_key_a, sym_key_b);
    assert_eq!(
        derive_walletconnect_session_topic(&sym_key_a),
        hash_walletconnect_key(&sym_key_a)
    );
}

#[test]
fn rejects_all_zero_x25519_shared_secret() {
    assert!(matches!(
        derive_walletconnect_session_sym_key(&[1u8; 32], &[0u8; 32]),
        Err(WalletConnectError::Crypto)
    ));
}
