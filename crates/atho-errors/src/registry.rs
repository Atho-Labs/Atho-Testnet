use crate::{AthoErrorCategory, AthoErrorCode, AthoErrorDescriptor, AthoSeverity};

macro_rules! descriptors {
    ($(
        $name:ident => (
            $code:literal,
            $category:ident,
            $severity:ident,
            $user_facing:expr,
            $consensus_critical:expr,
            $title:literal,
            $explanation:literal,
            $common_cause:literal,
            $suggested_fix:literal
        )
    ),+ $(,)?) => {
        $(
            pub const $name: AthoErrorDescriptor = AthoErrorDescriptor {
                code: AthoErrorCode::new($code),
                category: AthoErrorCategory::$category,
                severity: AthoSeverity::$severity,
                title: $title,
                explanation: $explanation,
                common_cause: $common_cause,
                suggested_fix: $suggested_fix,
                user_facing: $user_facing,
                consensus_critical: $consensus_critical,
            };
        )+

        pub const REGISTRY: &[&AthoErrorDescriptor] = &[
            $(&$name),+
        ];
    };
}

descriptors! {
    ADDR_INVALID_ALPHABET => ("ATHO-ADDR-001", Address, Error, true, false, "Invalid Base56 Alphabet", "The address contains bytes outside the Atho Base56 alphabet.", "The address text was copied incorrectly or contains unsupported characters.", "Check the address text and re-enter it without substitutions."),
    ADDR_INVALID_PREFIX => ("ATHO-ADDR-002", Address, Error, true, false, "Invalid Address Prefix", "The address prefix does not match the expected Atho network format.", "A mainnet, testnet, or regnet address was used in the wrong context.", "Use an address generated for the active network."),
    ADDR_INVALID_CHECKSUM => ("ATHO-ADDR-003", Address, Error, true, false, "Invalid Address Checksum", "The address checksum does not match the encoded payload.", "The address was truncated, modified, or pasted incorrectly.", "Verify the full address and paste it again from a trusted source."),

    CONS_INVALID_SUBSIDY_SCHEDULE => ("ATHO-CONS-001", Consensus, Critical, false, true, "Invalid Subsidy Schedule", "The block reward schedule could not be evaluated correctly.", "Consensus reward parameters or halving state are inconsistent.", "Verify the emission schedule constants and activation logic."),
    CONS_INVALID_POW_TARGET => ("ATHO-CONS-002", Consensus, Critical, false, true, "Invalid Proof-of-Work Target", "The proof-of-work target is malformed or outside the allowed consensus bounds.", "The block target field is corrupted or calculated incorrectly.", "Recompute the canonical target and verify difficulty encoding."),

    NET_INVALID_NETWORK_SELECTION => ("ATHO-NET-001", Network, Error, true, false, "Invalid Network Selection", "The requested Atho network mode is not recognized.", "An invalid network name was provided through the CLI or environment.", "Use one of atho-mainnet, atho-testnet, or atho-regnet."),
    NET_BLOCK_NETWORK_MISMATCH => ("ATHO-NET-002", Network, Critical, true, true, "Wrong Block Network", "The block belongs to a different Atho network than the active node.", "A mainnet, testnet, or regnet artifact was submitted to the wrong node.", "Submit the block to a node running the same network."),
    NET_INVALID_MAGIC => ("ATHO-NET-003", Network, Warning, false, false, "Invalid Network Magic", "The P2P message header does not match any known Atho network magic.", "A peer sent cross-network traffic or malformed bytes.", "Disconnect the peer and verify network isolation."),
    NET_GENESIS_MISMATCH => ("ATHO-NET-004", Network, Critical, false, true, "Genesis Mismatch", "The remote peer or local state does not match the expected Atho genesis hash.", "The node is connected to the wrong network or storage path.", "Use a network-specific data directory and verify the configured network."),
    NET_RULESET_MISMATCH => ("ATHO-NET-005", Network, Critical, false, true, "Ruleset Mismatch", "The peer announced a different consensus ruleset version than the local node.", "The nodes are running incompatible protocol or activation schedules.", "Upgrade or isolate nodes so they share the same ruleset."),
    NET_UNSUPPORTED_NETWORK => ("ATHO-NET-006", Network, Error, true, false, "Unsupported Network", "The requested Atho network or transport context is not supported by this code path.", "A caller passed an unsupported network identifier or transport boundary.", "Use a supported Atho network and verify the network-specific runtime."),

    BLK_EMPTY_BLOCK => ("ATHO-BLK-001", Block, Critical, false, true, "Empty Block", "A block must contain at least one transaction.", "The candidate block was assembled incorrectly.", "Ensure the coinbase transaction is present before validation."),
    BLK_BLOCK_TOO_LARGE => ("ATHO-BLK-002", Block, Critical, false, true, "Block Too Large", "The block exceeds Atho's configured size or weight limits.", "Too many transactions or oversized witnesses were included.", "Reduce block contents so size, weight, and vsize stay within limits."),
    BLK_MERKLE_ROOT_MISMATCH => ("ATHO-BLK-003", Block, Critical, false, true, "Merkle Root Mismatch", "The block header merkle root does not match the transaction list.", "Transactions changed after the header was built or were serialized incorrectly.", "Rebuild the merkle tree from the canonical transaction list."),
    BLK_WITNESS_ROOT_MISMATCH => ("ATHO-BLK-004", Block, Critical, false, true, "Witness Root Mismatch", "The witness root commitment does not match the block's transaction witnesses.", "Witness data changed after block assembly or commitment refs were built incorrectly.", "Recompute the witness root from the canonical witness payloads."),
    BLK_POW_INVALID => ("ATHO-BLK-005", Block, Critical, false, true, "Proof-of-Work Invalid", "The block hash does not satisfy the advertised difficulty target.", "The nonce search result is stale, malformed, or hashed with different header bytes.", "Rebuild the canonical header and rerun proof-of-work against the correct target."),
    BLK_PARENT_HASH_MISMATCH => ("ATHO-BLK-006", Block, Critical, false, true, "Previous Block Hash Mismatch", "The block does not connect to the expected parent hash.", "The block was built on the wrong tip or the parent field was mutated.", "Refresh the template and rebuild the block on the current canonical tip."),
    BLK_INVALID_HEIGHT => ("ATHO-BLK-007", Block, Critical, false, true, "Invalid Block Height", "The block height does not match the expected chain height.", "The header height field was built against the wrong parent or chain state.", "Refresh the tip and recompute the candidate block height."),
    BLK_INVALID_VERSION => ("ATHO-BLK-008", Block, Critical, false, true, "Invalid Block Version", "The block version is not supported at the current height.", "The template used an unsupported or inactive block version.", "Use the node-provided version for the active ruleset."),
    BLK_INVALID_TIMESTAMP => ("ATHO-BLK-009", Block, Critical, false, true, "Invalid Block Timestamp", "The block timestamp is outside Atho's accepted range.", "The timestamp is stale, too far in the future, or not monotonic enough.", "Rebuild the block with a fresh network-valid timestamp."),
    BLK_MULTIPLE_COINBASE => ("ATHO-BLK-010", Block, Critical, false, true, "Multiple Coinbase Transactions", "A block may contain only one coinbase transaction.", "Block assembly duplicated or misordered coinbase-like entries.", "Ensure the transaction list starts with exactly one coinbase."),
    BLK_INVALID_COINBASE => ("ATHO-BLK-011", Block, Critical, false, true, "Invalid Coinbase Transaction", "The coinbase transaction structure is invalid for the candidate block.", "The coinbase was serialized incorrectly or failed network-specific rules.", "Rebuild the coinbase using the canonical node template path."),
    BLK_COINBASE_REWARD_MISMATCH => ("ATHO-BLK-012", Block, Critical, false, true, "Coinbase Reward Mismatch", "The block pays a different reward than Atho consensus allows.", "Subsidy, fee totals, or payout distribution were computed incorrectly.", "Recalculate subsidy and total fees before constructing the coinbase."),
    BLK_DUPLICATE_TRANSACTION_ID => ("ATHO-BLK-013", Block, Critical, false, true, "Duplicate Transaction ID", "The block contains duplicate transactions.", "The same transaction was inserted twice during block assembly.", "Deduplicate the transaction list before building commitments."),
    BLK_TARGET_OUT_OF_BOUNDS => ("ATHO-BLK-014", Block, Critical, false, true, "Block Target Out of Bounds", "The block target field is outside Atho's allowed consensus range.", "Difficulty encoding or target serialization is wrong.", "Clamp or recalculate the target using the network difficulty rules."),

    TX_NO_INPUTS => ("ATHO-TX-001", Transaction, Critical, true, true, "Transaction Has No Inputs", "The transaction does not contain any spendable inputs.", "A non-coinbase transaction was created without references to previous outputs.", "Add at least one valid input or mark the transaction as coinbase where appropriate."),
    TX_NO_OUTPUTS => ("ATHO-TX-002", Transaction, Critical, true, true, "Transaction Has No Outputs", "The transaction does not contain any outputs.", "The transaction builder omitted all destinations.", "Add at least one output before signing the transaction."),
    TX_DUPLICATE_INPUT => ("ATHO-TX-003", Transaction, Critical, true, true, "Duplicate Transaction Input", "The transaction spends the same outpoint more than once.", "Input selection inserted the same UTXO multiple times.", "Deduplicate the input list before finalizing the transaction."),
    TX_ZERO_VALUE_OUTPUT => ("ATHO-TX-004", Transaction, Critical, true, true, "Zero-Value Output", "The transaction contains an output with zero atoms.", "A destination amount was left empty or rounded down to zero.", "Remove zero-value outputs or assign a valid amount."),
    TX_TOO_LARGE => ("ATHO-TX-005", Transaction, Critical, true, true, "Transaction Too Large", "The transaction exceeds Atho's size or weight limits.", "Too many inputs, outputs, or witness bytes were included.", "Reduce transaction complexity until it fits within protocol limits."),
    TX_INVALID_VERSION => ("ATHO-TX-006", Transaction, Critical, true, true, "Invalid Transaction Version", "The transaction version is not active at the current height.", "The transaction builder used an unsupported version number.", "Use a transaction version supported by the current ruleset."),
    TX_FEE_BELOW_MINIMUM => ("ATHO-TX-007", Transaction, Critical, true, true, "Fee Below Minimum", "The transaction fee is below Atho's minimum accepted amount for its virtual size.", "The fee calculator underpriced the transaction.", "Increase the fee so it meets the current minimum per-vbyte rule."),
    TX_FEE_MISMATCH => ("ATHO-TX-008", Transaction, Critical, false, true, "Fee Mismatch", "The transaction or block fee accounting does not balance.", "Inputs, outputs, or burned/miner/pool fee fields were tallied incorrectly.", "Recompute fee totals from canonical input and output values."),
    TX_MISSING_UTXO => ("ATHO-TX-009", Transaction, Critical, true, true, "Missing UTXO", "The transaction references an outpoint that is not available in the current UTXO set.", "The input was already spent, never existed, or belongs to another fork.", "Refresh wallet state and build the transaction from confirmed unspent outputs."),
    TX_INPUT_OWNERSHIP_MISMATCH => ("ATHO-TX-010", Transaction, Critical, true, true, "Input Ownership Mismatch", "The witness public key does not match the locking script being spent.", "The wrong key was used or the input was paired with the wrong witness.", "Sign each input with the private key that matches the referenced output."),
    TX_INSUFFICIENT_CONFIRMATIONS => ("ATHO-TX-011", Transaction, Critical, true, true, "Insufficient Confirmations", "The transaction spends an output that is not mature enough yet.", "A coinbase or newly created output was spent before the required confirmation depth.", "Wait for the required number of confirmations before spending the output."),
    TX_TOO_MANY_OUTPUTS => ("ATHO-TX-012", Transaction, Critical, true, true, "Too Many Outputs", "The transaction creates more outputs than Atho's standard anti-spam policy allows.", "A wallet batch or spam-shaped transaction exceeded the configured output cap.", "Reduce the output count or split the spend into smaller transactions."),
    TX_WRONG_POW_BITS => ("ATHO-TX-013", Transaction, Critical, true, true, "Wrong Transaction PoW Bits", "The transaction declares a wallet proof-of-work difficulty that does not match Atho policy.", "The wallet used the wrong fee, size, or output-count inputs when calculating transaction PoW.", "Recompute the required transaction PoW bits from the final signed transaction."),
    TX_INVALID_POW_NONCE => ("ATHO-TX-014", Transaction, Critical, true, true, "Invalid Transaction PoW Nonce", "The wallet transaction proof-of-work nonce does not satisfy the required SHA3-256 difficulty.", "The transaction changed after solving PoW or the nonce search used the wrong preimage.", "Rebuild the signed transaction and solve the wallet PoW again."),

    UTXO_MISSING => ("ATHO-UTXO-001", Utxo, Critical, false, true, "Missing UTXO Entry", "The requested UTXO entry does not exist in storage or chainstate.", "A spend referenced a pruned, missing, or never-created outpoint.", "Rebuild or resync the chainstate and verify the referenced outpoint."),
    UTXO_DUPLICATE => ("ATHO-UTXO-002", Utxo, Critical, false, true, "Duplicate UTXO Entry", "A new UTXO would overwrite an existing entry unexpectedly.", "Chainstate accounting attempted to create the same output twice.", "Inspect block connection logic for duplicate output creation."),

    MEM_MEMPOOL_CONFLICT => ("ATHO-MEM-001", Mempool, Error, true, false, "Mempool Conflict", "The transaction conflicts with an existing mempool spend or policy state.", "The same outpoint is already reserved by another pending transaction.", "Wait for the conflicting transaction to confirm or replace it intentionally."),
    MEM_DUST_OUTPUT => ("ATHO-MEM-002", Mempool, Error, true, false, "Dust Output Rejected", "The transaction creates an output below Atho's relay dust floor.", "A wallet or sender attempted to create an output smaller than the 1,000-atom relay minimum.", "Raise every spendable output to at least 1,000 atoms or combine the value into fees."),

    SIG_INVALID_WITNESS => ("ATHO-SIG-001", Signature, Critical, true, true, "Invalid Witness", "The transaction witness is missing, malformed, or fails Falcon validation.", "Witness bytes, signature length, or canonical signing data are inconsistent.", "Rebuild the witness using the correct Falcon public key and signing digest."),
    SIG_WITNESS_INPUT_REF_MISMATCH => ("ATHO-SIG-002", Signature, Critical, false, true, "Witness Input Reference Mismatch", "The witness input reference commitments do not match the canonical transaction witness.", "The witness reference shortcuts were built against different signature or txid bytes.", "Recompute witness input references from the canonical transaction and signature."),
    SIG_BACKEND_UNAVAILABLE => ("ATHO-SIG-003", Signature, Error, false, false, "Cryptography Backend Unavailable", "The Falcon cryptography backend could not provide the requested operation.", "Randomness or low-level crypto primitives were unavailable.", "Verify the runtime environment and linked Falcon backend."),
    SIG_INVALID_KEY_LENGTH => ("ATHO-SIG-004", Signature, Error, true, false, "Invalid Falcon Key Length", "A Falcon public or private key has the wrong length for Atho Falcon-512.", "A key was truncated, encoded incorrectly, or imported from an incompatible source.", "Use a full Falcon-512 key generated by Atho-compatible tooling."),
    SIG_CRYPTO_OPERATION_FAILED => ("ATHO-SIG-005", Signature, Error, false, false, "Falcon Operation Failed", "A Falcon signing or verification operation failed unexpectedly.", "The cryptography backend rejected the operation or internal state was invalid.", "Retry with canonical inputs and verify the key material."),

    HASH_CHECKSUM_MISMATCH => ("ATHO-HASH-001", Hash, Error, false, false, "Checksum Mismatch", "The computed checksum does not match the serialized payload.", "A message or payload was corrupted in transit or mutated after encoding.", "Re-encode the payload and verify the framing checksum logic."),

    RPC_METHOD_NOT_FOUND => ("ATHO-RPC-001", Rpc, Error, true, false, "RPC Method Not Found", "The requested RPC method is not supported by this endpoint.", "The client called a method that the node runtime does not implement here.", "Call a supported RPC method for the current runtime."),
    RPC_INVALID_REQUEST => ("ATHO-RPC-002", Rpc, Error, true, false, "Invalid RPC Request", "The RPC request was malformed or missing required data.", "The request body was incomplete, invalid, or sent to the wrong handler.", "Review the RPC request shape and required parameters."),
    RPC_INTERNAL => ("ATHO-RPC-003", Rpc, Error, true, false, "Internal RPC Error", "The node encountered an internal runtime error while handling the RPC request.", "A storage, runtime, or networking dependency failed behind the RPC layer.", "Inspect the node logs using the reported code and module."),
    RPC_VALIDATION => ("ATHO-RPC-004", Rpc, Error, true, false, "RPC Validation Error", "The request was rejected because it violated Atho validation rules.", "The submitted transaction or block failed a consensus or policy check.", "Review the attached error code and rebuild the submitted artifact."),
    RPC_TRANSPORT_IO => ("ATHO-RPC-005", Rpc, Error, true, false, "RPC Transport I/O Failure", "The RPC transport failed while opening, reading, or writing the connection.", "The RPC socket is unavailable, refused, or closed unexpectedly.", "Confirm the node is running and the RPC address is reachable."),
    RPC_SERIALIZATION => ("ATHO-RPC-006", Rpc, Error, true, false, "RPC Serialization Failure", "The RPC transport could not encode or decode the JSON payload.", "The peer sent invalid JSON or the payload shape no longer matches the schema.", "Check the RPC protocol version and payload format."),
    RPC_EMPTY_RESPONSE => ("ATHO-RPC-007", Rpc, Error, true, false, "Empty RPC Response", "The RPC transport returned no response line.", "The server closed the connection before sending a valid response.", "Check the node log for an earlier transport or runtime failure."),
    RPC_MESSAGE_TOO_LARGE => ("ATHO-RPC-008", Rpc, Error, true, false, "RPC Message Too Large", "The RPC payload exceeded the configured message size limit.", "The request or response body is unexpectedly large or malformed.", "Reduce the payload size or inspect the remote endpoint for framing errors."),
    RPC_QT_UNEXPECTED => ("ATHO-RPC-009", Rpc, Error, true, false, "Unexpected RPC Response", "The Qt client received an RPC response that does not match the expected call shape.", "The client and node disagree about the requested method or response variant.", "Verify both binaries are built from compatible code."),

    P2P_UNKNOWN_COMMAND => ("ATHO-P2P-001", P2p, Warning, false, false, "Unknown Message Command", "The P2P frame command name is not recognized.", "A peer sent malformed or unsupported protocol bytes.", "Ignore or disconnect the peer and verify protocol compatibility."),
    P2P_PAYLOAD_TOO_LARGE => ("ATHO-P2P-002", P2p, Warning, false, false, "Payload Too Large", "The P2P payload exceeds Atho's configured message size limits.", "A peer sent oversized inventory, headers, or malformed framing.", "Drop the message and enforce the network message size limit."),
    P2P_MALFORMED_PAYLOAD => ("ATHO-P2P-003", P2p, Warning, false, false, "Malformed P2P Payload", "The P2P payload could not be decoded into the expected message shape.", "The peer sent truncated, invalid, or corrupted serialized bytes.", "Disconnect or penalize the peer and inspect the failing message type."),
    P2P_UNEXPECTED_PAYLOAD => ("ATHO-P2P-004", P2p, Warning, false, false, "Unexpected P2P Payload", "The message payload does not match the announced command.", "The peer serialized the wrong message type for the frame command.", "Verify command-to-payload mapping and reject the message."),
    P2P_UNSUPPORTED_PROTOCOL => ("ATHO-P2P-005", P2p, Warning, false, false, "Unsupported Protocol Version", "The peer announced a protocol version outside Atho's supported range.", "The remote node is outdated or incompatible.", "Upgrade the peer or restrict connections to supported versions."),
    P2P_USER_AGENT_TOO_LONG => ("ATHO-P2P-006", P2p, Warning, false, false, "User Agent Too Long", "The peer user agent string exceeds the allowed protocol limit.", "The peer sent an oversized or malformed version message.", "Reject the version message and enforce the configured limit."),
    P2P_TOO_MANY_PEER_ADDRESSES => ("ATHO-P2P-007", P2p, Warning, false, false, "Too Many Peer Addresses", "The peer advertised more addresses than one message may carry.", "Address gossip exceeded the configured protocol limits.", "Reject or trim the message and review peer gossip behavior."),
    P2P_TOO_MANY_INVENTORY => ("ATHO-P2P-008", P2p, Warning, false, false, "Too Many Inventory Entries", "The peer sent too many inventory vectors in a single message.", "The inventory relay exceeded the configured entry cap.", "Split inventory relay into smaller batches."),
    P2P_TOO_MANY_HEADERS => ("ATHO-P2P-009", P2p, Warning, false, false, "Too Many Headers", "The peer sent more headers than Atho allows in one response.", "Header sync batching exceeded the configured limit.", "Reduce header batch size and enforce peer limits."),
    P2P_INVALID_HEADERS_SEQUENCE => ("ATHO-P2P-010", P2p, Warning, false, false, "Invalid Headers Sequence", "The advertised header chain is not internally linked or ordered correctly.", "The peer sent a non-contiguous or malformed header response.", "Reject the header batch and request a fresh locator-based sync."),
    P2P_PEER_BOOK_FULL => ("ATHO-P2P-011", P2p, Warning, false, false, "Peer Book Full", "The local peer address book cannot accept more entries.", "Peer gossip reached the configured storage capacity.", "Purge stale addresses or increase the peer book limit deliberately."),
    P2P_HANDSHAKE_INCOMPLETE => ("ATHO-P2P-012", P2p, Warning, false, false, "Handshake Incomplete", "The peer attempted to continue protocol traffic before completing the handshake.", "The version/verack exchange did not finish correctly.", "Require a full handshake before processing additional messages."),
    P2P_TOO_MANY_LOCATORS => ("ATHO-P2P-013", P2p, Warning, false, false, "Too Many Locator Hashes", "The peer sent more block locator hashes than Atho allows.", "A getheaders request exceeded the configured locator limit.", "Reject the locator list and request a bounded retry."),
    P2P_INVALID_COMPACT_BLOCK => ("ATHO-P2P-014", P2p, Warning, false, false, "Invalid Compact Block", "The compact block frame could not be reconstructed correctly.", "Short IDs, prefilled transactions, or indexes are inconsistent.", "Request the missing transactions or fall back to a full block."),
    P2P_PEER_ALREADY_CONNECTED => ("ATHO-P2P-015", P2p, Warning, true, false, "Peer Already Connected", "The connection manager already has a session for this remote peer.", "Duplicate inbound or outbound connection state was created.", "Reuse the existing session or disconnect the duplicate."),
    P2P_UNKNOWN_PEER => ("ATHO-P2P-016", P2p, Warning, false, false, "Unknown Peer Session", "A message or event referenced a peer that is not tracked locally.", "The session was already dropped or never established.", "Refresh the peer map before routing the event."),
    P2P_BANNED_PEER => ("ATHO-P2P-017", P2p, Warning, true, false, "Banned Peer", "The peer is currently banned and cannot connect.", "Prior misbehavior or repeated protocol violations triggered the ban list.", "Wait for the ban to expire or clear it after investigation."),
    P2P_INBOUND_LIMIT => ("ATHO-P2P-018", P2p, Warning, true, false, "Inbound Peer Limit Reached", "The node has reached its configured inbound peer limit.", "Too many inbound peers are already connected.", "Increase the limit intentionally or retry later."),
    P2P_OUTBOUND_LIMIT => ("ATHO-P2P-019", P2p, Warning, true, false, "Outbound Peer Limit Reached", "The node has reached its configured outbound peer limit.", "Too many outbound peers are already being maintained.", "Reduce manual peers or raise the outbound limit deliberately."),
    P2P_MESSAGE_TOO_SHORT => ("ATHO-P2P-020", P2p, Warning, false, false, "P2P Message Too Short", "The incoming frame is shorter than the Atho wire header.", "A peer sent a truncated or malformed network frame.", "Drop the frame and inspect the remote sender."),
    P2P_IO_FAILURE => ("ATHO-P2P-021", P2p, Error, true, false, "P2P I/O Failure", "A socket or stream operation failed while handling a peer connection.", "The peer disconnected unexpectedly or the local transport encountered an I/O error.", "Retry the connection and inspect the local network stack or peer health."),

    DB_IO => ("ATHO-DB-001", Database, Error, true, false, "Storage I/O Error", "A filesystem I/O operation failed while reading or writing Atho state.", "The data directory is missing, locked, or unavailable.", "Check filesystem permissions, free space, and the configured data path."),
    DB_LMDB => ("ATHO-DB-002", Database, Error, true, false, "LMDB Failure", "LMDB returned an error while accessing Atho storage.", "The environment, transaction, or map state is invalid or unavailable.", "Inspect the LMDB environment, file permissions, and map sizing."),
    DB_PATH_UNAVAILABLE => ("ATHO-DB-003", Database, Error, true, false, "Database Path Unavailable", "The configured Atho storage path could not be resolved or opened.", "The data directory does not exist or cannot be accessed.", "Create the path or point the node at a valid writable directory."),
    DB_CORRUPT_DATA => ("ATHO-DB-004", Database, Fatal, true, false, "Corrupt Storage Data", "Persisted Atho state could not be decoded safely.", "Database bytes are corrupted, truncated, or from an incompatible schema.", "Restore from a trusted snapshot or rebuild the database from canonical blocks."),
    DB_PERSISTED_GENESIS_MISMATCH => ("ATHO-DB-005", Database, Fatal, true, false, "Persisted Genesis Mismatch", "The stored genesis state does not match the active network.", "A mainnet, testnet, or regnet database was reused for another network.", "Use a network-specific data directory and resync the chainstate."),
    DB_PERSISTED_TIP_MISMATCH => ("ATHO-DB-006", Database, Fatal, true, false, "Persisted Tip Mismatch", "The stored tip header does not match persisted chainstate metadata.", "The database was interrupted mid-commit or contains inconsistent state.", "Recover from snapshot or rebuild the local chainstate."),
    DB_INCOMPLETE_BLOCK_HISTORY => ("ATHO-DB-007", Database, Fatal, true, false, "Incomplete Block History", "Persisted block history is missing entries needed to reconstruct the tip.", "Rollback history was pruned or written incompletely.", "Restore the full retained history or resync the node."),
    DB_CROSS_NETWORK_REPLAY => ("ATHO-DB-008", Database, Fatal, true, false, "Cross-Network Replay Detected", "Stored state appears to mix artifacts from different Atho networks.", "The wrong database root was reused or copied between networks.", "Isolate network-specific data directories and wipe the mixed state."),
    DB_LEGACY_STORAGE_LAYOUT => ("ATHO-DB-009", Database, Fatal, true, false, "Legacy Storage Layout Detected", "The node found an older incompatible storage layout.", "A previous release stored data under a different schema or directory plan.", "Migrate the database or wipe and rebuild with the current layout."),
    DB_SCHEMA_VERSION_MISMATCH => ("ATHO-DB-010", Database, Fatal, true, false, "Schema Version Mismatch", "The persisted database schema version does not match the running binary.", "A different Atho release created the database with an incompatible schema.", "Run the matching binary or perform the required migration."),
    DB_NO_BLOCK_TO_DISCONNECT => ("ATHO-DB-011", Database, Critical, false, true, "No Block To Disconnect", "Chainstate rollback was requested with no connected block to undo.", "Rollback bookkeeping lost sync with the active chain.", "Inspect rollback callers and retained undo history."),
    DB_EMPTY_BRANCH => ("ATHO-DB-012", Database, Critical, false, true, "Empty Branch", "A candidate branch contained no blocks to evaluate.", "Fork selection was called with an empty branch set.", "Verify branch assembly before chain selection."),
    DB_FORK_POINT_UNAVAILABLE => ("ATHO-DB-013", Database, Critical, false, true, "Fork Point Unavailable", "The branch fork point is outside retained canonical history.", "History pruning removed the rollback anchor needed for reorg.", "Increase retained history or resync before replaying the branch."),
    DB_INVALID_BRANCH_SEQUENCE => ("ATHO-DB-014", Database, Critical, false, true, "Invalid Branch Sequence", "The candidate branch is not internally ordered from fork point to tip.", "Blocks were buffered out of order or against different parents.", "Rebuild the branch in strict parent-child order."),
    DB_BRANCH_NOT_PREFERRED => ("ATHO-DB-015", Database, Warning, false, true, "Branch Not Preferred", "The candidate branch does not beat the current canonical chain.", "Chainwork, height, or tie-break rules prefer the existing tip.", "Keep the current chain or present a stronger candidate branch."),
    DB_ROLLBACK_FAILURE => ("ATHO-DB-016", Database, Fatal, false, true, "Rollback Failure", "The node could not restore canonical state during rollback.", "Undo data is missing, corrupt, or inconsistent with chainstate.", "Stop the node and recover the chainstate from a trusted snapshot."),

    MINE_CANCELLED => ("ATHO-MINE-001", Mining, Warning, true, false, "Mining Cancelled", "The mining backend stopped because the current job was cancelled.", "A newer template arrived or shutdown was requested.", "Restart mining with a fresh template."),
    MINE_BACKEND_FAILURE => ("ATHO-MINE-002", Mining, Error, true, false, "Mining Backend Failure", "The selected mining backend failed to process the current job.", "The backend is unavailable, misconfigured, or returned invalid output.", "Inspect the backend-specific code and retry with CPU fallback if needed."),
    MINE_GPU_FEATURE_DISABLED => ("ATHO-MINE-101", Mining, Error, true, false, "GPU Feature Disabled", "GPU mining support is not enabled in this build.", "The binary was built without the gpu-native feature.", "Rebuild Atho with GPU support or use the CPU backend."),
    MINE_GPU_NOT_FOUND => ("ATHO-MINE-102", Mining, Error, true, false, "GPU Not Found", "No compatible real GPU device was detected for Atho mining.", "The system has no supported GPU runtime or the driver is unavailable.", "Install the correct GPU runtime or use Auto/CPU mode."),
    MINE_GPU_KERNEL_MISSING => ("ATHO-MINE-103", Mining, Error, true, false, "GPU Kernel Missing", "The GPU kernel source could not be located.", "The kernel file is missing or the configured path is wrong.", "Verify the kernel bundle and ATHO_GPU_KERNEL_PATH."),
    MINE_GPU_KERNEL_LOAD_FAILED => ("ATHO-MINE-104", Mining, Error, true, false, "GPU Kernel Load Failed", "The GPU kernel source file could not be read.", "The kernel path is unreadable, empty, or invalid.", "Check the kernel file path and file permissions."),
    MINE_GPU_KERNEL_BUILD_FAILED => ("ATHO-MINE-105", Mining, Error, true, false, "GPU Kernel Build Failed", "The GPU runtime failed to compile the mining kernel.", "Driver, runtime, or kernel source incompatibilities prevented compilation.", "Inspect the build log and adjust the runtime or kernel source."),
    MINE_GPU_CONTEXT_CREATE_FAILED => ("ATHO-MINE-106", Mining, Error, true, false, "GPU Context Create Failed", "The GPU runtime could not create a device context.", "The OpenCL, CUDA, or future runtime is misconfigured or unavailable.", "Verify the GPU driver installation and runtime libraries."),
    MINE_GPU_QUEUE_CREATE_FAILED => ("ATHO-MINE-107", Mining, Error, true, false, "GPU Queue Create Failed", "The GPU runtime could not create a command queue.", "The selected device or runtime rejected queue creation.", "Retry after verifying the GPU runtime and device health."),
    MINE_GPU_KERNEL_CREATE_FAILED => ("ATHO-MINE-108", Mining, Error, true, false, "GPU Kernel Create Failed", "The GPU runtime could not create the mining kernel instance.", "The compiled program does not export the expected kernel symbol.", "Verify the kernel name and compiled program contents."),
    MINE_GPU_BUFFER_ALLOC_FAILED => ("ATHO-MINE-109", Mining, Error, true, false, "GPU Buffer Allocation Failed", "The GPU runtime could not allocate required mining buffers.", "Device memory is exhausted or buffer sizing is invalid.", "Reduce batch size or free device memory before retrying."),
    MINE_GPU_INVALID_ARGUMENT => ("ATHO-MINE-110", Mining, Error, true, false, "GPU Invalid Argument", "The GPU mining request contains invalid lengths or parameters.", "Header bytes, nonce offset, target bytes, or buffers are malformed.", "Rebuild the mining job from the canonical node template."),
    MINE_GPU_BATCH_TOO_LARGE => ("ATHO-MINE-111", Mining, Error, true, false, "GPU Batch Too Large", "The requested GPU batch size exceeds the configured maximum.", "The batch size tuning is too aggressive for the current runtime.", "Reduce ATHO_GPU_BATCH_SIZE or increase the allowed cap deliberately."),
    MINE_GPU_NONCE_OVERFLOW => ("ATHO-MINE-112", Mining, Error, true, false, "GPU Nonce Overflow", "The requested nonce range would overflow the 64-bit nonce space.", "Start nonce and batch size extend past the maximum allowed nonce.", "Clamp the nonce range and request a fresh mining batch."),
    MINE_GPU_KERNEL_EXEC_FAILED => ("ATHO-MINE-113", Mining, Error, true, false, "GPU Kernel Execution Failed", "The GPU runtime failed while executing the mining kernel.", "The device aborted execution or the runtime reported a launch failure.", "Retry after checking GPU stability and kernel compatibility."),
    MINE_GPU_BUFFER_IO_FAILED => ("ATHO-MINE-114", Mining, Error, true, false, "GPU Buffer I/O Failed", "The GPU runtime failed while moving buffers to or from device memory.", "Driver, queue, or memory-transfer operations failed mid-job.", "Retry with a smaller batch size and verify device stability."),
    MINE_GPU_PROBE_FAILED => ("ATHO-MINE-115", Mining, Error, true, false, "GPU Probe Failed", "The GPU probe could not collect structured device information.", "Runtime enumeration failed before a mining session started.", "Inspect the GPU runtime installation and probe logs."),
    MINE_GPU_UNKNOWN => ("ATHO-MINE-116", Mining, Error, true, false, "Unknown GPU Failure", "The GPU backend returned an unknown failure.", "The native helper reported an unexpected or uncategorized error.", "Inspect the native helper logs and add a more specific error mapping."),
    MINE_GPU_SOLUTION_MISMATCH => ("ATHO-MINE-117", Mining, Critical, false, true, "GPU Solution Mismatch", "The GPU backend returned a nonce that does not match Atho's canonical CPU verification.", "The backend hashed different header bytes or returned a corrupted result.", "Treat the backend as faulty, reject the solution, and verify the kernel against CPU vectors."),

    LAUNCH_PUBLIC_RPC_DENIED => ("ATHO-LAUNCH-001", Launcher, Error, true, false, "Public RPC Bind Denied", "Atho refuses to bind RPC to a non-loopback address without explicit opt-in.", "ATHO_RPC_ALLOW_PUBLIC was not set and the configured RPC bind is public.", "Use a loopback RPC address or set ATHO_RPC_ALLOW_PUBLIC=1 intentionally."),
    LAUNCH_RPC_BIND_FAILED => ("ATHO-LAUNCH-002", Launcher, Error, true, false, "RPC Bind Failed", "The node could not bind its RPC listener.", "Another process is using the port or the address is invalid.", "Choose a free RPC address and verify port permissions."),
    LAUNCH_P2P_BIND_FAILED => ("ATHO-LAUNCH-003", Launcher, Error, true, false, "P2P Bind Failed", "The node could not bind its P2P listener.", "Another process is using the P2P port or the address is invalid.", "Choose a free P2P address and verify firewall or port settings."),
    LAUNCH_INVALID_PEER_ADDRESS => ("ATHO-LAUNCH-004", Launcher, Error, true, false, "Invalid Peer Address", "The supplied peer address is not a valid socket address.", "The peer string is malformed or missing a port.", "Use a valid host:port pair for manual peer configuration."),

    WALLET_INVALID_MNEMONIC_WORD_COUNT => ("ATHO-WALLET-001", Wallet, Error, true, false, "Invalid Mnemonic Word Count", "The mnemonic does not contain a supported number of words.", "The phrase is incomplete or uses an unsupported length.", "Use a 12, 24, or 48 word Atho mnemonic."),
    WALLET_INVALID_ENTROPY_LENGTH => ("ATHO-WALLET-002", Wallet, Error, true, false, "Invalid Mnemonic Entropy Length", "The provided entropy length does not match the requested mnemonic size.", "The entropy buffer is truncated or not sized for the selected mnemonic length.", "Provide entropy with the exact byte length required by the mnemonic format."),
    WALLET_INVALID_MNEMONIC_WORD => ("ATHO-WALLET-003", Wallet, Error, true, false, "Invalid Mnemonic Word", "The mnemonic contains a word outside Atho's approved wordlist.", "One or more mnemonic words were mistyped or come from another wordlist.", "Correct the invalid word using the Atho mnemonic wordlist."),
    WALLET_INVALID_MNEMONIC_CHECKSUM => ("ATHO-WALLET-004", Wallet, Error, true, false, "Mnemonic Checksum Mismatch", "The mnemonic checksum bits do not match the encoded entropy.", "The phrase was entered incorrectly or truncated.", "Re-enter the mnemonic exactly as generated."),
    WALLET_IO => ("ATHO-WALLET-005", Wallet, Error, true, false, "Wallet File I/O Error", "A wallet file could not be read or written.", "The wallet path is missing, locked, or lacks permissions.", "Verify the wallet path, file permissions, and free disk space."),
    WALLET_SERIALIZATION => ("ATHO-WALLET-006", Wallet, Error, false, false, "Wallet Serialization Failure", "The wallet state could not be encoded or decoded safely.", "The wallet file is corrupted or the serialized schema changed unexpectedly.", "Restore from a trusted backup or rebuild the wallet file."),
    WALLET_INVALID_HEADER => ("ATHO-WALLET-007", Wallet, Error, true, false, "Invalid Wallet Header", "The wallet datafile header is malformed or truncated.", "The wallet file is corrupt or not an Atho wallet datafile.", "Verify the file source and restore from backup if necessary."),
    WALLET_UNSUPPORTED_VERSION => ("ATHO-WALLET-008", Wallet, Error, true, false, "Unsupported Wallet Version", "The wallet datafile version is not supported by this build.", "The wallet file was created by a different incompatible release.", "Use a compatible Atho release or migrate the wallet file."),
    WALLET_UNSUPPORTED_ENCRYPTION_MODE => ("ATHO-WALLET-009", Wallet, Error, true, false, "Unsupported Wallet Encryption Mode", "The wallet uses an encryption mode that this build cannot read.", "The datafile header references an unknown or future encryption mode.", "Upgrade to a compatible build or re-export the wallet in a supported format."),
    WALLET_RANDOMNESS_FAILURE => ("ATHO-WALLET-010", Wallet, Fatal, false, false, "Wallet Randomness Failure", "Secure randomness could not be obtained for wallet encryption.", "The operating system randomness source failed or is unavailable.", "Fix the host entropy source before generating or encrypting wallet material."),
    WALLET_INVALID_PASSWORD => ("ATHO-WALLET-011", Wallet, Error, true, false, "Wallet Password Rejected", "The wallet password is wrong or the encrypted wallet file is corrupted.", "The password is incorrect or authenticated decryption failed.", "Retry with the correct password or restore the wallet from backup.")
}

pub fn registry_descriptor(code: &str) -> Option<&'static AthoErrorDescriptor> {
    REGISTRY
        .iter()
        .copied()
        .find(|descriptor| descriptor.code.as_str() == code)
}

pub fn render_markdown_registry() -> String {
    let mut out = String::from("# Atho Error Codes\n\n");
    out.push_str("This registry is generated from `crates/atho-errors` and is the canonical source for Atho error metadata.\n\n");
    for descriptor in REGISTRY {
        out.push_str(&format!("## {}\n", descriptor.code));
        out.push_str(&format!(
            "- Category: `{}`\n",
            descriptor.category.short_code()
        ));
        out.push_str(&format!("- Title: {}\n", descriptor.title));
        out.push_str(&format!("- Severity: `{}`\n", descriptor.severity.as_str()));
        out.push_str(&format!("- User Facing: `{}`\n", descriptor.user_facing));
        out.push_str(&format!(
            "- Consensus Critical: `{}`\n",
            descriptor.consensus_critical
        ));
        out.push_str(&format!("- Explanation: {}\n", descriptor.explanation));
        out.push_str(&format!("- Common Cause: {}\n", descriptor.common_cause));
        out.push_str(&format!(
            "- Suggested Fix: {}\n\n",
            descriptor.suggested_fix
        ));
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

pub fn render_json_registry() -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(REGISTRY)
}
