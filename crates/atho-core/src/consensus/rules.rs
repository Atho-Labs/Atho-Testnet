// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Consensus versioning and activation scheduling.
//!
//! These helpers translate a chain height into the block and transaction
//! versions the node must accept at that height.

/// Effective consensus versions active at a particular chain height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsensusRules {
    /// P2P and storage protocol version expected by this software line.
    pub protocol_version: u32,
    /// Logical ruleset version for consensus feature activation.
    pub ruleset_version: u32,
    /// Block header version miners must use at this height.
    pub block_version: u16,
    /// Transaction version accepted at this height.
    pub transaction_version: u16,
    /// Height where this ruleset became active.
    pub activation_height: u64,
}

/// A single scheduled ruleset activation entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledActivation {
    /// Stable human-readable activation label.
    pub name: &'static str,
    /// Ruleset version introduced by this activation.
    pub ruleset_version: u32,
    /// Required block version once the activation is live.
    pub block_version: u16,
    /// Required transaction version once the activation is live.
    pub transaction_version: u16,
    /// Height at which the activation becomes live, or `None` if dormant.
    pub activation_height: Option<u64>,
}

pub const PROTOCOL_VERSION: u32 = 1;
pub const STORAGE_SCHEMA_VERSION: u32 = 4;

pub const RULESET_VERSION_V1: u32 = 1;
pub const BLOCK_VERSION_V1: u16 = 1;
pub const TRANSACTION_VERSION_V1: u16 = 1;
pub const RULESET_V1_ACTIVATION_HEIGHT: u64 = 0;

pub const RULESET_VERSION_V2_PLACEHOLDER: u32 = 2;
pub const BLOCK_VERSION_V2_PLACEHOLDER: u16 = 2;
pub const TRANSACTION_VERSION_V2_PLACEHOLDER: u16 = 2;
pub const RULESET_V2_ACTIVATION_HEIGHT: Option<u64> = None;

pub const ACTIVE_RULESET_V1: ConsensusRules = ConsensusRules {
    protocol_version: PROTOCOL_VERSION,
    ruleset_version: RULESET_VERSION_V1,
    block_version: BLOCK_VERSION_V1,
    transaction_version: TRANSACTION_VERSION_V1,
    activation_height: RULESET_V1_ACTIVATION_HEIGHT,
};

pub const SCHEDULED_ACTIVATIONS: [ScheduledActivation; 2] = [
    ScheduledActivation {
        name: "atho-ruleset-v1",
        ruleset_version: RULESET_VERSION_V1,
        block_version: BLOCK_VERSION_V1,
        transaction_version: TRANSACTION_VERSION_V1,
        activation_height: Some(RULESET_V1_ACTIVATION_HEIGHT),
    },
    ScheduledActivation {
        name: "atho-ruleset-v2-placeholder",
        ruleset_version: RULESET_VERSION_V2_PLACEHOLDER,
        block_version: BLOCK_VERSION_V2_PLACEHOLDER,
        transaction_version: TRANSACTION_VERSION_V2_PLACEHOLDER,
        activation_height: RULESET_V2_ACTIVATION_HEIGHT,
    },
];

/// Returns the active consensus versions at `height` using the default schedule.
pub fn rules_at_height(height: u64) -> ConsensusRules {
    rules_at_height_with_schedule(height, &SCHEDULED_ACTIVATIONS)
}

/// Resolves the active ruleset at `height` for an arbitrary activation schedule.
///
/// The latest activation whose start height is less than or equal to `height`
/// wins. If no entry is active yet, the first schedule entry is treated as the
/// default baseline.
pub fn rules_at_height_with_schedule(
    height: u64,
    schedule: &[ScheduledActivation],
) -> ConsensusRules {
    let activation = schedule
        .iter()
        .filter_map(|activation| {
            activation
                .activation_height
                .filter(|activation_height| *activation_height <= height)
                .map(|activation_height| (*activation, activation_height))
        })
        .max_by_key(|(_, activation_height)| *activation_height)
        .map(|(activation, _)| activation);
    let activation = activation.unwrap_or_else(|| {
        schedule
            .first()
            .copied()
            .unwrap_or(SCHEDULED_ACTIVATIONS[0])
    });
    ConsensusRules {
        protocol_version: PROTOCOL_VERSION,
        ruleset_version: activation.ruleset_version,
        block_version: activation.block_version,
        transaction_version: activation.transaction_version,
        activation_height: activation
            .activation_height
            .unwrap_or(RULESET_V1_ACTIVATION_HEIGHT),
    }
}

/// Returns the canonical block version at `height`.
pub fn block_version_at_height(height: u64) -> u16 {
    rules_at_height(height).block_version
}

/// Returns the canonical block version for a custom activation schedule.
pub fn block_version_at_height_with_schedule(height: u64, schedule: &[ScheduledActivation]) -> u16 {
    rules_at_height_with_schedule(height, schedule).block_version
}

/// Returns the canonical transaction version at `height`.
pub fn transaction_version_at_height(height: u64) -> u16 {
    rules_at_height(height).transaction_version
}

/// Returns the canonical transaction version for a custom activation schedule.
pub fn transaction_version_at_height_with_schedule(
    height: u64,
    schedule: &[ScheduledActivation],
) -> u16 {
    rules_at_height_with_schedule(height, schedule).transaction_version
}

/// Returns the logical ruleset version active at `height`.
pub fn ruleset_version_at_height(height: u64) -> u32 {
    rules_at_height(height).ruleset_version
}

/// Reports whether `version` is valid for a block at `height`.
pub fn is_supported_block_version(version: u16, height: u64) -> bool {
    block_version_at_height(height) == version
}

/// Reports whether `version` is valid for a block at `height` under `schedule`.
pub fn is_supported_block_version_with_schedule(
    version: u16,
    height: u64,
    schedule: &[ScheduledActivation],
) -> bool {
    block_version_at_height_with_schedule(height, schedule) == version
}

/// Reports whether `version` is valid for a transaction at `height`.
pub fn is_supported_transaction_version(version: u16, height: u64) -> bool {
    transaction_version_at_height(height) == version
}

/// Reports whether `version` is valid for a transaction at `height` under `schedule`.
pub fn is_supported_transaction_version_with_schedule(
    version: u16,
    height: u64,
    schedule: &[ScheduledActivation],
) -> bool {
    transaction_version_at_height_with_schedule(height, schedule) == version
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruleset_v1_is_active_from_genesis() {
        let genesis_rules = rules_at_height(0);
        let future_rules = rules_at_height(1_000_000);
        assert_eq!(genesis_rules.protocol_version, 1);
        assert_eq!(genesis_rules.ruleset_version, RULESET_VERSION_V1);
        assert_eq!(genesis_rules.block_version, BLOCK_VERSION_V1);
        assert_eq!(genesis_rules.transaction_version, TRANSACTION_VERSION_V1);
        assert_eq!(future_rules, genesis_rules);
    }

    #[test]
    fn future_activation_placeholder_is_explicit_but_inactive() {
        assert_eq!(RULESET_V2_ACTIVATION_HEIGHT, None);
        assert_eq!(SCHEDULED_ACTIVATIONS.len(), 2);
        assert!(is_supported_block_version(BLOCK_VERSION_V1, 10));
        assert!(!is_supported_block_version(
            BLOCK_VERSION_V2_PLACEHOLDER,
            10
        ));
        assert!(is_supported_transaction_version(TRANSACTION_VERSION_V1, 10));
        assert!(!is_supported_transaction_version(
            TRANSACTION_VERSION_V2_PLACEHOLDER,
            10
        ));
    }

    #[test]
    fn scheduled_v2_activation_switches_versions_at_the_exact_height() {
        let schedule = [
            ScheduledActivation {
                name: "atho-ruleset-v1",
                ruleset_version: RULESET_VERSION_V1,
                block_version: BLOCK_VERSION_V1,
                transaction_version: TRANSACTION_VERSION_V1,
                activation_height: Some(0),
            },
            ScheduledActivation {
                name: "atho-ruleset-v2",
                ruleset_version: RULESET_VERSION_V2_PLACEHOLDER,
                block_version: BLOCK_VERSION_V2_PLACEHOLDER,
                transaction_version: TRANSACTION_VERSION_V2_PLACEHOLDER,
                activation_height: Some(12),
            },
        ];

        assert_eq!(
            rules_at_height_with_schedule(11, &schedule).ruleset_version,
            1
        );
        assert_eq!(
            rules_at_height_with_schedule(12, &schedule).ruleset_version,
            2
        );
        assert_eq!(block_version_at_height_with_schedule(11, &schedule), 1);
        assert_eq!(block_version_at_height_with_schedule(12, &schedule), 2);
        assert_eq!(
            transaction_version_at_height_with_schedule(11, &schedule),
            1
        );
        assert_eq!(
            transaction_version_at_height_with_schedule(12, &schedule),
            2
        );
        assert!(is_supported_block_version_with_schedule(1, 11, &schedule));
        assert!(!is_supported_block_version_with_schedule(2, 11, &schedule));
        assert!(is_supported_block_version_with_schedule(2, 12, &schedule));
        assert!(is_supported_transaction_version_with_schedule(
            2, 12, &schedule
        ));
    }
}
