#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsensusRules {
    pub protocol_version: u32,
    pub ruleset_version: u32,
    pub block_version: u16,
    pub transaction_version: u16,
    pub activation_height: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledActivation {
    pub name: &'static str,
    pub ruleset_version: u32,
    pub block_version: u16,
    pub transaction_version: u16,
    pub activation_height: Option<u64>,
}

pub const PROTOCOL_VERSION: u32 = 1;
pub const STORAGE_SCHEMA_VERSION: u32 = 3;

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

pub fn rules_at_height(height: u64) -> ConsensusRules {
    rules_at_height_with_schedule(height, &SCHEDULED_ACTIVATIONS)
}

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

pub fn block_version_at_height(height: u64) -> u16 {
    rules_at_height(height).block_version
}

pub fn block_version_at_height_with_schedule(height: u64, schedule: &[ScheduledActivation]) -> u16 {
    rules_at_height_with_schedule(height, schedule).block_version
}

pub fn transaction_version_at_height(height: u64) -> u16 {
    rules_at_height(height).transaction_version
}

pub fn transaction_version_at_height_with_schedule(
    height: u64,
    schedule: &[ScheduledActivation],
) -> u16 {
    rules_at_height_with_schedule(height, schedule).transaction_version
}

pub fn ruleset_version_at_height(height: u64) -> u32 {
    rules_at_height(height).ruleset_version
}

pub fn is_supported_block_version(version: u16, height: u64) -> bool {
    block_version_at_height(height) == version
}

pub fn is_supported_block_version_with_schedule(
    version: u16,
    height: u64,
    schedule: &[ScheduledActivation],
) -> bool {
    block_version_at_height_with_schedule(height, schedule) == version
}

pub fn is_supported_transaction_version(version: u16, height: u64) -> bool {
    transaction_version_at_height(height) == version
}

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
