use trezor_client::TrezorMessage;
use trezor_client::protos::MessageType;
use trezor_client::transport::ProtoMessage;
use zeroize::Zeroize;

use super::super::HardwareDerivationError;
use super::passphrase::{TrezorPassphraseState, TrezorPinMatrixProvider};

pub(super) fn trezor_call_raw_with_interactions<S: TrezorMessage>(
    client: &mut trezor_client::Trezor,
    message: S,
    passphrase: &mut TrezorPassphraseState,
    pin_matrix_provider: Option<&TrezorPinMatrixProvider>,
) -> Result<ProtoMessage, HardwareDerivationError> {
    let response = client.call_raw(message)?;
    trezor_handle_raw_interactions(client, response, passphrase, pin_matrix_provider)
}

fn trezor_handle_raw_interactions(
    client: &mut trezor_client::Trezor,
    mut response: ProtoMessage,
    passphrase: &mut TrezorPassphraseState,
    pin_matrix_provider: Option<&TrezorPinMatrixProvider>,
) -> Result<ProtoMessage, HardwareDerivationError> {
    loop {
        match response.message_type() {
            MessageType::MessageType_Failure => {
                let failure: trezor_client::protos::Failure = trezor_decode_message(response)?;
                return Err(trezor_client::Error::FailureResponse(failure).into());
            }
            MessageType::MessageType_ButtonRequest => {
                let _request: trezor_client::protos::ButtonRequest =
                    trezor_decode_message(response)?;
                response = client.call_raw(trezor_client::protos::ButtonAck::new())?;
            }
            MessageType::MessageType_PinMatrixRequest => {
                let request: trezor_client::protos::PinMatrixRequest =
                    trezor_decode_message(response)?;
                let Some(provider) = pin_matrix_provider else {
                    return Err(HardwareDerivationError::UnsupportedTrezorPinMatrix);
                };
                let mut pin = provider(request.type_().into())?;
                let mut ack = trezor_client::protos::PinMatrixAck::new();
                ack.set_pin(pin.as_str().to_owned());
                pin.zeroize();
                response = client.call_raw(ack)?;
            }
            MessageType::MessageType_PassphraseRequest => {
                let request: trezor_client::protos::PassphraseRequest =
                    trezor_decode_message(response)?;
                let ack = passphrase.next_passphrase_ack(request._on_device())?;
                response = client.call_raw(ack)?;
            }
            _ => return Ok(response),
        }
    }
}

pub(super) fn trezor_decode_message<M: protobuf::Message>(
    message: ProtoMessage,
) -> Result<M, HardwareDerivationError> {
    message.into_message().map_err(|_| {
        HardwareDerivationError::UnexpectedHardwareResponse(
            "Trezor returned a malformed protobuf response",
        )
    })
}
