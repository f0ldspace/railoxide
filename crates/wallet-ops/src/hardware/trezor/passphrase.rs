use std::fmt;
use std::sync::{Arc, OnceLock};

use trezor_client::TrezorMessage;
use trezor_client::client::TrezorResponse;
use zeroize::{Zeroize, Zeroizing};

use super::super::HardwareDerivationError;
use crate::vault::TrezorPassphraseMode;

pub type TrezorPinMatrixProvider = Arc<
    dyn Fn(TrezorPinMatrixRequestKind) -> Result<Zeroizing<String>, HardwareDerivationError>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrezorPinMatrixRequestKind {
    Current,
    NewFirst,
    NewSecond,
    WipeCodeFirst,
    WipeCodeSecond,
}

impl From<trezor_client::protos::pin_matrix_request::PinMatrixRequestType>
    for TrezorPinMatrixRequestKind
{
    fn from(value: trezor_client::protos::pin_matrix_request::PinMatrixRequestType) -> Self {
        match value {
            trezor_client::protos::pin_matrix_request::PinMatrixRequestType::PinMatrixRequestType_Current => Self::Current,
            trezor_client::protos::pin_matrix_request::PinMatrixRequestType::PinMatrixRequestType_NewFirst => Self::NewFirst,
            trezor_client::protos::pin_matrix_request::PinMatrixRequestType::PinMatrixRequestType_NewSecond => Self::NewSecond,
            trezor_client::protos::pin_matrix_request::PinMatrixRequestType::PinMatrixRequestType_WipeCodeFirst => Self::WipeCodeFirst,
            trezor_client::protos::pin_matrix_request::PinMatrixRequestType::PinMatrixRequestType_WipeCodeSecond => Self::WipeCodeSecond,
        }
    }
}

pub(super) struct TrezorPassphraseState {
    pub(super) mode: TrezorPassphraseMode,
    pub(super) app_passphrase: Option<Zeroizing<String>>,
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct ZeroizingPassphraseAck {
    inner: trezor_client::protos::PassphraseAck,
}

impl ZeroizingPassphraseAck {
    fn new() -> Self {
        Self {
            inner: trezor_client::protos::PassphraseAck::new(),
        }
    }

    fn set_on_device(&mut self, value: bool) {
        self.inner.set_on_device(value);
    }

    fn set_passphrase(&mut self, passphrase: String) {
        self.inner.set_passphrase(passphrase);
    }

    #[cfg(test)]
    pub(super) fn has_on_device(&self) -> bool {
        self.inner.has_on_device()
    }

    #[cfg(test)]
    pub(super) fn on_device(&self) -> bool {
        self.inner.on_device()
    }

    #[cfg(test)]
    pub(super) fn has_passphrase(&self) -> bool {
        self.inner.has_passphrase()
    }

    #[cfg(test)]
    pub(super) fn passphrase(&self) -> &str {
        self.inner.passphrase()
    }
}

impl fmt::Debug for ZeroizingPassphraseAck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PassphraseAck")
            .field(
                "passphrase",
                &self.inner.passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field("on_device", &self.inner.on_device)
            .finish_non_exhaustive()
    }
}

impl Drop for ZeroizingPassphraseAck {
    fn drop(&mut self) {
        if let Some(passphrase) = self.inner.passphrase.as_mut() {
            passphrase.zeroize();
        }
        self.inner.clear_passphrase();
    }
}

impl protobuf::Message for ZeroizingPassphraseAck {
    const NAME: &'static str = <trezor_client::protos::PassphraseAck as protobuf::Message>::NAME;

    fn is_initialized(&self) -> bool {
        self.inner.is_initialized()
    }

    fn merge_from(&mut self, is: &mut protobuf::CodedInputStream<'_>) -> protobuf::Result<()> {
        self.inner.merge_from(is)
    }

    fn write_to_with_cached_sizes(
        &self,
        os: &mut protobuf::CodedOutputStream<'_>,
    ) -> protobuf::Result<()> {
        self.inner.write_to_with_cached_sizes(os)
    }

    fn compute_size(&self) -> u64 {
        self.inner.compute_size()
    }

    fn cached_size(&self) -> u32 {
        self.inner.cached_size()
    }

    fn special_fields(&self) -> &protobuf::SpecialFields {
        self.inner.special_fields()
    }

    fn mut_special_fields(&mut self) -> &mut protobuf::SpecialFields {
        self.inner.mut_special_fields()
    }

    fn new() -> Self {
        Self {
            inner: trezor_client::protos::PassphraseAck::new(),
        }
    }

    fn default_instance() -> &'static Self {
        static INSTANCE: OnceLock<ZeroizingPassphraseAck> = OnceLock::new();
        INSTANCE.get_or_init(Self::new)
    }
}

impl TrezorMessage for ZeroizingPassphraseAck {
    const MESSAGE_TYPE: trezor_client::protos::MessageType =
        trezor_client::protos::MessageType::MessageType_PassphraseAck;
}

impl Default for TrezorPassphraseState {
    fn default() -> Self {
        Self {
            mode: TrezorPassphraseMode::NoPassphrase,
            app_passphrase: None,
        }
    }
}

impl TrezorPassphraseState {
    pub(super) fn set_mode(&mut self, mode: TrezorPassphraseMode) {
        self.mode = mode;
        if mode != TrezorPassphraseMode::EnterInApp {
            self.clear_app_passphrase();
        }
    }

    pub(super) fn set_app_passphrase(&mut self, passphrase: String) {
        self.set_app_passphrase_zeroizing(Zeroizing::new(passphrase));
    }

    pub(super) fn set_app_passphrase_zeroizing(&mut self, passphrase: Zeroizing<String>) {
        self.mode = TrezorPassphraseMode::EnterInApp;
        self.app_passphrase = Some(passphrase);
    }

    pub(super) fn clear_app_passphrase(&mut self) {
        if let Some(mut passphrase) = self.app_passphrase.take() {
            passphrase.zeroize();
        }
    }

    pub(super) fn next_passphrase_ack(
        &mut self,
        device_requires_on_device: bool,
    ) -> Result<ZeroizingPassphraseAck, HardwareDerivationError> {
        let mut ack = ZeroizingPassphraseAck::new();
        if device_requires_on_device {
            self.clear_app_passphrase();
            return Ok(ack);
        }
        match self.mode {
            TrezorPassphraseMode::NoPassphrase => ack.set_passphrase(String::new()),
            TrezorPassphraseMode::EnterOnTrezor => ack.set_on_device(true),
            TrezorPassphraseMode::EnterInApp => {
                let Some(mut passphrase) = self.app_passphrase.take() else {
                    return Err(HardwareDerivationError::MissingTrezorAppPassphrase);
                };
                ack.set_passphrase(passphrase.as_str().to_owned());
                passphrase.zeroize();
            }
        }
        Ok(ack)
    }
}

pub(super) fn handle_trezor_interaction<T, R: TrezorMessage>(
    response: TrezorResponse<'_, T, R>,
    passphrase: &mut TrezorPassphraseState,
    pin_matrix_provider: Option<&TrezorPinMatrixProvider>,
) -> Result<T, HardwareDerivationError> {
    match response {
        TrezorResponse::Ok(value) => Ok(value),
        TrezorResponse::Failure(failure) => {
            Err(trezor_client::Error::FailureResponse(failure).into())
        }
        TrezorResponse::ButtonRequest(request) => {
            handle_trezor_interaction(request.ack()?, passphrase, pin_matrix_provider)
        }
        TrezorResponse::PinMatrixRequest(request) => {
            let Some(provider) = pin_matrix_provider else {
                return Err(HardwareDerivationError::UnsupportedTrezorPinMatrix);
            };
            let mut pin = provider(request.request_type().into())?;
            let next = request.ack_pin(pin.as_str().to_owned())?;
            pin.zeroize();
            handle_trezor_interaction(next, passphrase, pin_matrix_provider)
        }
        TrezorResponse::PassphraseRequest(request) => {
            let ack = passphrase.next_passphrase_ack(request.on_device())?;
            handle_trezor_interaction(
                request.client.call(ack, request.result_handler)?,
                passphrase,
                pin_matrix_provider,
            )
        }
    }
}
