use super::{helpers::*, relay::*, render::*, requests::*, *};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use wallet_ops::WalletConnectNamespaceProposal;
use wallet_ops::hardware::{
    HardwareDerivationDescriptor, HardwareDeviceKind, HardwarePublicAccountDescriptor,
    HardwareViewAccessKey, HardwareWalletSyncIntent, parse_bip32_path,
};
use wallet_ops::vault::{
    KdfParams, PublicAccountScope, WalletConnectApprovedNamespace, WalletConnectSessionKeys,
    WalletSource,
};

mod fixtures;

mod attention;

mod relay_protocol;

mod relay_worker;

mod pairing;

mod proposal;

mod approval;

mod request_progress;

mod request_expiry;

mod session_lifecycle;

mod display;
