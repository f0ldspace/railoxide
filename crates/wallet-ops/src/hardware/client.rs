use std::collections::VecDeque;

use async_trait::async_trait;

use super::derivation::{
    HardwareDerivationDescriptor, HardwareOperationOutput, SyntheticRailgunEntropy,
    synthetic_entropy_from_hardware_output,
};
use super::error::HardwareDerivationError;

#[async_trait(?Send)]
pub trait HardwareDerivationClient {
    async fn derive_hardware_output(
        &mut self,
        descriptor: &HardwareDerivationDescriptor,
    ) -> Result<HardwareOperationOutput, HardwareDerivationError>;

    async fn derive_synthetic_entropy(
        &mut self,
        descriptor: &HardwareDerivationDescriptor,
    ) -> Result<SyntheticRailgunEntropy, HardwareDerivationError> {
        let output = self.derive_hardware_output(descriptor).await?;
        synthetic_entropy_from_hardware_output(descriptor, output)
    }
}

pub struct MockHardwareDerivationClient {
    outputs: VecDeque<HardwareOperationOutput>,
}

impl MockHardwareDerivationClient {
    #[must_use]
    pub fn new(outputs: impl IntoIterator<Item = [u8; 32]>) -> Self {
        Self {
            outputs: outputs
                .into_iter()
                .map(HardwareOperationOutput::new)
                .collect(),
        }
    }
}

#[async_trait(?Send)]
impl HardwareDerivationClient for MockHardwareDerivationClient {
    async fn derive_hardware_output(
        &mut self,
        descriptor: &HardwareDerivationDescriptor,
    ) -> Result<HardwareOperationOutput, HardwareDerivationError> {
        descriptor.validate()?;
        self.outputs
            .pop_front()
            .ok_or(HardwareDerivationError::MissingMockOutput)
    }
}
