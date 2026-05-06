# Atho Error Codes

This registry is generated from `crates/atho-errors` and is the canonical source for Atho error metadata.

## ATHO-ADDR-001
- Category: `ADDR`
- Title: Invalid Base56 Alphabet
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The address contains bytes outside the Atho Base56 alphabet.
- Common Cause: The address text was copied incorrectly or contains unsupported characters.
- Suggested Fix: Check the address text and re-enter it without substitutions.

## ATHO-ADDR-002
- Category: `ADDR`
- Title: Invalid Address Prefix
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The address prefix does not match the expected Atho network format.
- Common Cause: A mainnet, testnet, or regnet address was used in the wrong context.
- Suggested Fix: Use an address generated for the active network.

## ATHO-ADDR-003
- Category: `ADDR`
- Title: Invalid Address Checksum
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The address checksum does not match the encoded payload.
- Common Cause: The address was truncated, modified, or pasted incorrectly.
- Suggested Fix: Verify the full address and paste it again from a trusted source.

## ATHO-CONS-001
- Category: `CONS`
- Title: Invalid Subsidy Schedule
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block reward schedule could not be evaluated correctly.
- Common Cause: Consensus reward parameters or halving state are inconsistent.
- Suggested Fix: Verify the emission schedule constants and activation logic.

## ATHO-CONS-002
- Category: `CONS`
- Title: Invalid Proof-of-Work Target
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The proof-of-work target is malformed or outside the allowed consensus bounds.
- Common Cause: The block target field is corrupted or calculated incorrectly.
- Suggested Fix: Recompute the canonical target and verify difficulty encoding.

## ATHO-NET-001
- Category: `NET`
- Title: Invalid Network Selection
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The requested Atho network mode is not recognized.
- Common Cause: An invalid network name was provided through the CLI or environment.
- Suggested Fix: Use one of atho-mainnet, atho-testnet, or atho-regnet.

## ATHO-NET-002
- Category: `NET`
- Title: Wrong Block Network
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The block belongs to a different Atho network than the active node.
- Common Cause: A mainnet, testnet, or regnet artifact was submitted to the wrong node.
- Suggested Fix: Submit the block to a node running the same network.

## ATHO-NET-003
- Category: `NET`
- Title: Invalid Network Magic
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The P2P message header does not match any known Atho network magic.
- Common Cause: A peer sent cross-network traffic or malformed bytes.
- Suggested Fix: Disconnect the peer and verify network isolation.

## ATHO-NET-004
- Category: `NET`
- Title: Genesis Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The remote peer or local state does not match the expected Atho genesis hash.
- Common Cause: The node is connected to the wrong network or storage path.
- Suggested Fix: Use a network-specific data directory and verify the configured network.

## ATHO-NET-005
- Category: `NET`
- Title: Ruleset Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The peer announced a different consensus ruleset version than the local node.
- Common Cause: The nodes are running incompatible protocol or activation schedules.
- Suggested Fix: Upgrade or isolate nodes so they share the same ruleset.

## ATHO-NET-006
- Category: `NET`
- Title: Unsupported Network
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The requested Atho network or transport context is not supported by this code path.
- Common Cause: A caller passed an unsupported network identifier or transport boundary.
- Suggested Fix: Use a supported Atho network and verify the network-specific runtime.

## ATHO-BLK-001
- Category: `BLK`
- Title: Empty Block
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: A block must contain at least one transaction.
- Common Cause: The candidate block was assembled incorrectly.
- Suggested Fix: Ensure the coinbase transaction is present before validation.

## ATHO-BLK-002
- Category: `BLK`
- Title: Block Too Large
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block exceeds Atho's configured size or weight limits.
- Common Cause: Too many transactions or oversized witnesses were included.
- Suggested Fix: Reduce block contents so size, weight, and vsize stay within limits.

## ATHO-BLK-003
- Category: `BLK`
- Title: Merkle Root Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block header merkle root does not match the transaction list.
- Common Cause: Transactions changed after the header was built or were serialized incorrectly.
- Suggested Fix: Rebuild the merkle tree from the canonical transaction list.

## ATHO-BLK-004
- Category: `BLK`
- Title: Witness Root Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The witness root commitment does not match the block's transaction witnesses.
- Common Cause: Witness data changed after block assembly or commitment refs were built incorrectly.
- Suggested Fix: Recompute the witness root from the canonical witness payloads.

## ATHO-BLK-005
- Category: `BLK`
- Title: Proof-of-Work Invalid
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block hash does not satisfy the advertised difficulty target.
- Common Cause: The nonce search result is stale, malformed, or hashed with different header bytes.
- Suggested Fix: Rebuild the canonical header and rerun proof-of-work against the correct target.

## ATHO-BLK-006
- Category: `BLK`
- Title: Previous Block Hash Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block does not connect to the expected parent hash.
- Common Cause: The block was built on the wrong tip or the parent field was mutated.
- Suggested Fix: Refresh the template and rebuild the block on the current canonical tip.

## ATHO-BLK-007
- Category: `BLK`
- Title: Invalid Block Height
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block height does not match the expected chain height.
- Common Cause: The header height field was built against the wrong parent or chain state.
- Suggested Fix: Refresh the tip and recompute the candidate block height.

## ATHO-BLK-008
- Category: `BLK`
- Title: Invalid Block Version
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block version is not supported at the current height.
- Common Cause: The template used an unsupported or inactive block version.
- Suggested Fix: Use the node-provided version for the active ruleset.

## ATHO-BLK-009
- Category: `BLK`
- Title: Invalid Block Timestamp
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block timestamp is outside Atho's accepted range.
- Common Cause: The timestamp is stale, too far in the future, or not monotonic enough.
- Suggested Fix: Rebuild the block with a fresh network-valid timestamp.

## ATHO-BLK-010
- Category: `BLK`
- Title: Multiple Coinbase Transactions
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: A block may contain only one coinbase transaction.
- Common Cause: Block assembly duplicated or misordered coinbase-like entries.
- Suggested Fix: Ensure the transaction list starts with exactly one coinbase.

## ATHO-BLK-011
- Category: `BLK`
- Title: Invalid Coinbase Transaction
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The coinbase transaction structure is invalid for the candidate block.
- Common Cause: The coinbase was serialized incorrectly or failed network-specific rules.
- Suggested Fix: Rebuild the coinbase using the canonical node template path.

## ATHO-BLK-012
- Category: `BLK`
- Title: Coinbase Reward Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block pays a different reward than Atho consensus allows.
- Common Cause: Subsidy, fee totals, or payout distribution were computed incorrectly.
- Suggested Fix: Recalculate subsidy and total fees before constructing the coinbase.

## ATHO-BLK-013
- Category: `BLK`
- Title: Duplicate Transaction ID
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block contains duplicate transactions.
- Common Cause: The same transaction was inserted twice during block assembly.
- Suggested Fix: Deduplicate the transaction list before building commitments.

## ATHO-BLK-014
- Category: `BLK`
- Title: Block Target Out of Bounds
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The block target field is outside Atho's allowed consensus range.
- Common Cause: Difficulty encoding or target serialization is wrong.
- Suggested Fix: Clamp or recalculate the target using the network difficulty rules.

## ATHO-TX-001
- Category: `TX`
- Title: Transaction Has No Inputs
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction does not contain any spendable inputs.
- Common Cause: A non-coinbase transaction was created without references to previous outputs.
- Suggested Fix: Add at least one valid input or mark the transaction as coinbase where appropriate.

## ATHO-TX-002
- Category: `TX`
- Title: Transaction Has No Outputs
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction does not contain any outputs.
- Common Cause: The transaction builder omitted all destinations.
- Suggested Fix: Add at least one output before signing the transaction.

## ATHO-TX-003
- Category: `TX`
- Title: Duplicate Transaction Input
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction spends the same outpoint more than once.
- Common Cause: Input selection inserted the same UTXO multiple times.
- Suggested Fix: Deduplicate the input list before finalizing the transaction.

## ATHO-TX-004
- Category: `TX`
- Title: Zero-Value Output
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction contains an output with zero atoms.
- Common Cause: A destination amount was left empty or rounded down to zero.
- Suggested Fix: Remove zero-value outputs or assign a valid amount.

## ATHO-TX-005
- Category: `TX`
- Title: Transaction Too Large
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction exceeds Atho's size or weight limits.
- Common Cause: Too many inputs, outputs, or witness bytes were included.
- Suggested Fix: Reduce transaction complexity until it fits within protocol limits.

## ATHO-TX-006
- Category: `TX`
- Title: Invalid Transaction Version
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction version is not active at the current height.
- Common Cause: The transaction builder used an unsupported version number.
- Suggested Fix: Use a transaction version supported by the current ruleset.

## ATHO-TX-007
- Category: `TX`
- Title: Fee Below Minimum
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction fee is below Atho's minimum accepted amount for its virtual size.
- Common Cause: The fee calculator underpriced the transaction.
- Suggested Fix: Increase the fee so it meets the current minimum per-vbyte rule.

## ATHO-TX-008
- Category: `TX`
- Title: Fee Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The transaction or block fee accounting does not balance.
- Common Cause: Inputs, outputs, or burned/miner/pool fee fields were tallied incorrectly.
- Suggested Fix: Recompute fee totals from canonical input and output values.

## ATHO-TX-009
- Category: `TX`
- Title: Missing UTXO
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction references an outpoint that is not available in the current UTXO set.
- Common Cause: The input was already spent, never existed, or belongs to another fork.
- Suggested Fix: Refresh wallet state and build the transaction from confirmed unspent outputs.

## ATHO-TX-010
- Category: `TX`
- Title: Input Ownership Mismatch
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The witness public key does not match the locking script being spent.
- Common Cause: The wrong key was used or the input was paired with the wrong witness.
- Suggested Fix: Sign each input with the private key that matches the referenced output.

## ATHO-TX-011
- Category: `TX`
- Title: Insufficient Confirmations
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction spends an output that is not mature enough yet.
- Common Cause: A coinbase or newly created output was spent before the required confirmation depth.
- Suggested Fix: Wait for the required number of confirmations before spending the output.

## ATHO-TX-012
- Category: `TX`
- Title: Too Many Outputs
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction creates more outputs than Atho's standard anti-spam policy allows.
- Common Cause: A wallet batch or spam-shaped transaction exceeded the configured output cap.
- Suggested Fix: Reduce the output count or split the spend into smaller transactions.

## ATHO-TX-013
- Category: `TX`
- Title: Wrong Transaction PoW Bits
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction declares a wallet proof-of-work difficulty that does not match Atho policy.
- Common Cause: The wallet used the wrong fee, size, or output-count inputs when calculating transaction PoW.
- Suggested Fix: Recompute the required transaction PoW bits from the final signed transaction.

## ATHO-TX-014
- Category: `TX`
- Title: Invalid Transaction PoW Nonce
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The wallet transaction proof-of-work nonce does not satisfy the required SHA3-256 difficulty.
- Common Cause: The transaction changed after solving PoW or the nonce search used the wrong preimage.
- Suggested Fix: Rebuild the signed transaction and solve the wallet PoW again.

## ATHO-UTXO-001
- Category: `UTXO`
- Title: Missing UTXO Entry
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The requested UTXO entry does not exist in storage or chainstate.
- Common Cause: A spend referenced a pruned, missing, or never-created outpoint.
- Suggested Fix: Rebuild or resync the chainstate and verify the referenced outpoint.

## ATHO-UTXO-002
- Category: `UTXO`
- Title: Duplicate UTXO Entry
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: A new UTXO would overwrite an existing entry unexpectedly.
- Common Cause: Chainstate accounting attempted to create the same output twice.
- Suggested Fix: Inspect block connection logic for duplicate output creation.

## ATHO-MEM-001
- Category: `MEM`
- Title: Mempool Conflict
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The transaction conflicts with an existing mempool spend or policy state.
- Common Cause: The same outpoint is already reserved by another pending transaction.
- Suggested Fix: Wait for the conflicting transaction to confirm or replace it intentionally.

## ATHO-MEM-002
- Category: `MEM`
- Title: Output Below Minimum Rejected
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The transaction creates an output below Atho's 1,000-atom minimum output rule.
- Common Cause: A wallet or sender attempted to create an output smaller than the 1,000-atom relay minimum.
- Suggested Fix: Raise every spendable output to at least 1,000 atoms or combine the value into fees.

## ATHO-SIG-001
- Category: `SIG`
- Title: Invalid Witness
- Severity: `critical`
- User Facing: `true`
- Consensus Critical: `true`
- Explanation: The transaction witness is missing, malformed, or fails Falcon validation.
- Common Cause: Witness bytes, signature length, or canonical signing data are inconsistent.
- Suggested Fix: Rebuild the witness using the correct Falcon public key and signing digest.

## ATHO-SIG-002
- Category: `SIG`
- Title: Witness Input Reference Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The witness input reference commitments do not match the canonical transaction witness.
- Common Cause: The witness reference shortcuts were built against different signature or txid bytes.
- Suggested Fix: Recompute witness input references from the canonical transaction and signature.

## ATHO-SIG-003
- Category: `SIG`
- Title: Cryptography Backend Unavailable
- Severity: `error`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The Falcon cryptography backend could not provide the requested operation.
- Common Cause: Randomness or low-level crypto primitives were unavailable.
- Suggested Fix: Verify the runtime environment and linked Falcon backend.

## ATHO-SIG-004
- Category: `SIG`
- Title: Invalid Falcon Key Length
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: A Falcon public or private key has the wrong length for Atho Falcon-512.
- Common Cause: A key was truncated, encoded incorrectly, or imported from an incompatible source.
- Suggested Fix: Use a full Falcon-512 key generated by Atho-compatible tooling.

## ATHO-SIG-005
- Category: `SIG`
- Title: Falcon Operation Failed
- Severity: `error`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: A Falcon signing or verification operation failed unexpectedly.
- Common Cause: The cryptography backend rejected the operation or internal state was invalid.
- Suggested Fix: Retry with canonical inputs and verify the key material.

## ATHO-HASH-001
- Category: `HASH`
- Title: Checksum Mismatch
- Severity: `error`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The computed checksum does not match the serialized payload.
- Common Cause: A message or payload was corrupted in transit or mutated after encoding.
- Suggested Fix: Re-encode the payload and verify the framing checksum logic.

## ATHO-RPC-001
- Category: `RPC`
- Title: RPC Method Not Found
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The requested RPC method is not supported by this endpoint.
- Common Cause: The client called a method that the node runtime does not implement here.
- Suggested Fix: Call a supported RPC method for the current runtime.

## ATHO-RPC-002
- Category: `RPC`
- Title: Invalid RPC Request
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The RPC request was malformed or missing required data.
- Common Cause: The request body was incomplete, invalid, or sent to the wrong handler.
- Suggested Fix: Review the RPC request shape and required parameters.

## ATHO-RPC-003
- Category: `RPC`
- Title: Internal RPC Error
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The node encountered an internal runtime error while handling the RPC request.
- Common Cause: A storage, runtime, or networking dependency failed behind the RPC layer.
- Suggested Fix: Inspect the node logs using the reported code and module.

## ATHO-RPC-004
- Category: `RPC`
- Title: RPC Validation Error
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The request was rejected because it violated Atho validation rules.
- Common Cause: The submitted transaction or block failed a consensus or policy check.
- Suggested Fix: Review the attached error code and rebuild the submitted artifact.

## ATHO-RPC-005
- Category: `RPC`
- Title: RPC Transport I/O Failure
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The RPC transport failed while opening, reading, or writing the connection.
- Common Cause: The RPC socket is unavailable, refused, or closed unexpectedly.
- Suggested Fix: Confirm the node is running and the RPC address is reachable.

## ATHO-RPC-006
- Category: `RPC`
- Title: RPC Serialization Failure
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The RPC transport could not encode or decode the JSON payload.
- Common Cause: The peer sent invalid JSON or the payload shape no longer matches the schema.
- Suggested Fix: Check the RPC protocol version and payload format.

## ATHO-RPC-007
- Category: `RPC`
- Title: Empty RPC Response
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The RPC transport returned no response line.
- Common Cause: The server closed the connection before sending a valid response.
- Suggested Fix: Check the node log for an earlier transport or runtime failure.

## ATHO-RPC-008
- Category: `RPC`
- Title: RPC Message Too Large
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The RPC payload exceeded the configured message size limit.
- Common Cause: The request or response body is unexpectedly large or malformed.
- Suggested Fix: Reduce the payload size or inspect the remote endpoint for framing errors.

## ATHO-RPC-009
- Category: `RPC`
- Title: Unexpected RPC Response
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The Qt client received an RPC response that does not match the expected call shape.
- Common Cause: The client and node disagree about the requested method or response variant.
- Suggested Fix: Verify both binaries are built from compatible code.

## ATHO-P2P-001
- Category: `P2P`
- Title: Unknown Message Command
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The P2P frame command name is not recognized.
- Common Cause: A peer sent malformed or unsupported protocol bytes.
- Suggested Fix: Ignore or disconnect the peer and verify protocol compatibility.

## ATHO-P2P-002
- Category: `P2P`
- Title: Payload Too Large
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The P2P payload exceeds Atho's configured message size limits.
- Common Cause: A peer sent oversized inventory, headers, or malformed framing.
- Suggested Fix: Drop the message and enforce the network message size limit.

## ATHO-P2P-003
- Category: `P2P`
- Title: Malformed P2P Payload
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The P2P payload could not be decoded into the expected message shape.
- Common Cause: The peer sent truncated, invalid, or corrupted serialized bytes.
- Suggested Fix: Disconnect or penalize the peer and inspect the failing message type.

## ATHO-P2P-004
- Category: `P2P`
- Title: Unexpected P2P Payload
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The message payload does not match the announced command.
- Common Cause: The peer serialized the wrong message type for the frame command.
- Suggested Fix: Verify command-to-payload mapping and reject the message.

## ATHO-P2P-005
- Category: `P2P`
- Title: Unsupported Protocol Version
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The peer announced a protocol version outside Atho's supported range.
- Common Cause: The remote node is outdated or incompatible.
- Suggested Fix: Upgrade the peer or restrict connections to supported versions.

## ATHO-P2P-006
- Category: `P2P`
- Title: User Agent Too Long
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The peer user agent string exceeds the allowed protocol limit.
- Common Cause: The peer sent an oversized or malformed version message.
- Suggested Fix: Reject the version message and enforce the configured limit.

## ATHO-P2P-007
- Category: `P2P`
- Title: Too Many Peer Addresses
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The peer advertised more addresses than one message may carry.
- Common Cause: Address gossip exceeded the configured protocol limits.
- Suggested Fix: Reject or trim the message and review peer gossip behavior.

## ATHO-P2P-008
- Category: `P2P`
- Title: Too Many Inventory Entries
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The peer sent too many inventory vectors in a single message.
- Common Cause: The inventory relay exceeded the configured entry cap.
- Suggested Fix: Split inventory relay into smaller batches.

## ATHO-P2P-009
- Category: `P2P`
- Title: Too Many Headers
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The peer sent more headers than Atho allows in one response.
- Common Cause: Header sync batching exceeded the configured limit.
- Suggested Fix: Reduce header batch size and enforce peer limits.

## ATHO-P2P-010
- Category: `P2P`
- Title: Invalid Headers Sequence
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The advertised header chain is not internally linked or ordered correctly.
- Common Cause: The peer sent a non-contiguous or malformed header response.
- Suggested Fix: Reject the header batch and request a fresh locator-based sync.

## ATHO-P2P-011
- Category: `P2P`
- Title: Peer Book Full
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The local peer address book cannot accept more entries.
- Common Cause: Peer gossip reached the configured storage capacity.
- Suggested Fix: Purge stale addresses or increase the peer book limit deliberately.

## ATHO-P2P-012
- Category: `P2P`
- Title: Handshake Incomplete
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The peer attempted to continue protocol traffic before completing the handshake.
- Common Cause: The version/verack exchange did not finish correctly.
- Suggested Fix: Require a full handshake before processing additional messages.

## ATHO-P2P-013
- Category: `P2P`
- Title: Too Many Locator Hashes
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The peer sent more block locator hashes than Atho allows.
- Common Cause: A getheaders request exceeded the configured locator limit.
- Suggested Fix: Reject the locator list and request a bounded retry.

## ATHO-P2P-014
- Category: `P2P`
- Title: Invalid Compact Block
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The compact block frame could not be reconstructed correctly.
- Common Cause: Short IDs, prefilled transactions, or indexes are inconsistent.
- Suggested Fix: Request the missing transactions or fall back to a full block.

## ATHO-P2P-015
- Category: `P2P`
- Title: Peer Already Connected
- Severity: `warning`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The connection manager already has a session for this remote peer.
- Common Cause: Duplicate inbound or outbound connection state was created.
- Suggested Fix: Reuse the existing session or disconnect the duplicate.

## ATHO-P2P-016
- Category: `P2P`
- Title: Unknown Peer Session
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: A message or event referenced a peer that is not tracked locally.
- Common Cause: The session was already dropped or never established.
- Suggested Fix: Refresh the peer map before routing the event.

## ATHO-P2P-017
- Category: `P2P`
- Title: Banned Peer
- Severity: `warning`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The peer is currently banned and cannot connect.
- Common Cause: Prior misbehavior or repeated protocol violations triggered the ban list.
- Suggested Fix: Wait for the ban to expire or clear it after investigation.

## ATHO-P2P-018
- Category: `P2P`
- Title: Inbound Peer Limit Reached
- Severity: `warning`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The node has reached its configured inbound peer limit.
- Common Cause: Too many inbound peers are already connected.
- Suggested Fix: Increase the limit intentionally or retry later.

## ATHO-P2P-019
- Category: `P2P`
- Title: Outbound Peer Limit Reached
- Severity: `warning`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The node has reached its configured outbound peer limit.
- Common Cause: Too many outbound peers are already being maintained.
- Suggested Fix: Reduce manual peers or raise the outbound limit deliberately.

## ATHO-P2P-020
- Category: `P2P`
- Title: P2P Message Too Short
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The incoming frame is shorter than the Atho wire header.
- Common Cause: A peer sent a truncated or malformed network frame.
- Suggested Fix: Drop the frame and inspect the remote sender.

## ATHO-P2P-021
- Category: `P2P`
- Title: P2P I/O Failure
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: A socket or stream operation failed while handling a peer connection.
- Common Cause: The peer disconnected unexpectedly or the local transport encountered an I/O error.
- Suggested Fix: Retry the connection and inspect the local network stack or peer health.

## ATHO-DB-001
- Category: `DB`
- Title: Storage I/O Error
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: A filesystem I/O operation failed while reading or writing Atho state.
- Common Cause: The data directory is missing, locked, or unavailable.
- Suggested Fix: Check filesystem permissions, free space, and the configured data path.

## ATHO-DB-002
- Category: `DB`
- Title: LMDB Failure
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: LMDB returned an error while accessing Atho storage.
- Common Cause: The environment, transaction, or map state is invalid or unavailable.
- Suggested Fix: Inspect the LMDB environment, file permissions, and map sizing.

## ATHO-DB-003
- Category: `DB`
- Title: Database Path Unavailable
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The configured Atho storage path could not be resolved or opened.
- Common Cause: The data directory does not exist or cannot be accessed.
- Suggested Fix: Create the path or point the node at a valid writable directory.

## ATHO-DB-004
- Category: `DB`
- Title: Corrupt Storage Data
- Severity: `fatal`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: Persisted Atho state could not be decoded safely.
- Common Cause: Database bytes are corrupted, truncated, or from an incompatible schema.
- Suggested Fix: Restore from a trusted snapshot or rebuild the database from canonical blocks.

## ATHO-DB-005
- Category: `DB`
- Title: Persisted Genesis Mismatch
- Severity: `fatal`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The stored genesis state does not match the active network.
- Common Cause: A mainnet, testnet, or regnet database was reused for another network.
- Suggested Fix: Use a network-specific data directory and resync the chainstate.

## ATHO-DB-006
- Category: `DB`
- Title: Persisted Tip Mismatch
- Severity: `fatal`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The stored tip header does not match persisted chainstate metadata.
- Common Cause: The database was interrupted mid-commit or contains inconsistent state.
- Suggested Fix: Recover from snapshot or rebuild the local chainstate.

## ATHO-DB-007
- Category: `DB`
- Title: Incomplete Block History
- Severity: `fatal`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: Persisted block history is missing entries needed to reconstruct the tip.
- Common Cause: Rollback history was pruned or written incompletely.
- Suggested Fix: Restore the full retained history or resync the node.

## ATHO-DB-008
- Category: `DB`
- Title: Cross-Network Replay Detected
- Severity: `fatal`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: Stored state appears to mix artifacts from different Atho networks.
- Common Cause: The wrong database root was reused or copied between networks.
- Suggested Fix: Isolate network-specific data directories and wipe the mixed state.

## ATHO-DB-009
- Category: `DB`
- Title: Legacy Storage Layout Detected
- Severity: `fatal`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The node found an older incompatible storage layout.
- Common Cause: A previous release stored data under a different schema or directory plan.
- Suggested Fix: Migrate the database or wipe and rebuild with the current layout.

## ATHO-DB-010
- Category: `DB`
- Title: Schema Version Mismatch
- Severity: `fatal`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The persisted database schema version does not match the running binary.
- Common Cause: A different Atho release created the database with an incompatible schema.
- Suggested Fix: Run the matching binary or perform the required migration.

## ATHO-DB-011
- Category: `DB`
- Title: No Block To Disconnect
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: Chainstate rollback was requested with no connected block to undo.
- Common Cause: Rollback bookkeeping lost sync with the active chain.
- Suggested Fix: Inspect rollback callers and retained undo history.

## ATHO-DB-012
- Category: `DB`
- Title: Empty Branch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: A candidate branch contained no blocks to evaluate.
- Common Cause: Fork selection was called with an empty branch set.
- Suggested Fix: Verify branch assembly before chain selection.

## ATHO-DB-013
- Category: `DB`
- Title: Fork Point Unavailable
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The branch fork point is outside retained canonical history.
- Common Cause: History pruning removed the rollback anchor needed for reorg.
- Suggested Fix: Increase retained history or resync before replaying the branch.

## ATHO-DB-014
- Category: `DB`
- Title: Invalid Branch Sequence
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The candidate branch is not internally ordered from fork point to tip.
- Common Cause: Blocks were buffered out of order or against different parents.
- Suggested Fix: Rebuild the branch in strict parent-child order.

## ATHO-DB-015
- Category: `DB`
- Title: Branch Not Preferred
- Severity: `warning`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The candidate branch does not beat the current canonical chain.
- Common Cause: Chainwork, height, or tie-break rules prefer the existing tip.
- Suggested Fix: Keep the current chain or present a stronger candidate branch.

## ATHO-DB-016
- Category: `DB`
- Title: Rollback Failure
- Severity: `fatal`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The node could not restore canonical state during rollback.
- Common Cause: Undo data is missing, corrupt, or inconsistent with chainstate.
- Suggested Fix: Stop the node and recover the chainstate from a trusted snapshot.

## ATHO-MINE-001
- Category: `MINE`
- Title: Mining Cancelled
- Severity: `warning`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The mining backend stopped because the current job was cancelled.
- Common Cause: A newer template arrived or shutdown was requested.
- Suggested Fix: Restart mining with a fresh template.

## ATHO-MINE-002
- Category: `MINE`
- Title: Mining Backend Failure
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The selected mining backend failed to process the current job.
- Common Cause: The backend is unavailable, misconfigured, or returned invalid output.
- Suggested Fix: Inspect the backend-specific code and retry with CPU fallback if needed.

## ATHO-MINE-101
- Category: `MINE`
- Title: GPU Feature Disabled
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: GPU mining support is not enabled in this build.
- Common Cause: The binary was built without the gpu-native feature.
- Suggested Fix: Rebuild Atho with GPU support or use the CPU backend.

## ATHO-MINE-102
- Category: `MINE`
- Title: GPU Not Found
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: No compatible real GPU device was detected for Atho mining.
- Common Cause: The system has no supported GPU runtime or the driver is unavailable.
- Suggested Fix: Install the correct GPU runtime or use Auto/CPU mode.

## ATHO-MINE-103
- Category: `MINE`
- Title: GPU Kernel Missing
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU kernel source could not be located.
- Common Cause: The kernel file is missing or the configured path is wrong.
- Suggested Fix: Verify the kernel bundle and ATHO_GPU_KERNEL_PATH.

## ATHO-MINE-104
- Category: `MINE`
- Title: GPU Kernel Load Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU kernel source file could not be read.
- Common Cause: The kernel path is unreadable, empty, or invalid.
- Suggested Fix: Check the kernel file path and file permissions.

## ATHO-MINE-105
- Category: `MINE`
- Title: GPU Kernel Build Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU runtime failed to compile the mining kernel.
- Common Cause: Driver, runtime, or kernel source incompatibilities prevented compilation.
- Suggested Fix: Inspect the build log and adjust the runtime or kernel source.

## ATHO-MINE-106
- Category: `MINE`
- Title: GPU Context Create Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU runtime could not create a device context.
- Common Cause: The OpenCL, CUDA, or future runtime is misconfigured or unavailable.
- Suggested Fix: Verify the GPU driver installation and runtime libraries.

## ATHO-MINE-107
- Category: `MINE`
- Title: GPU Queue Create Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU runtime could not create a command queue.
- Common Cause: The selected device or runtime rejected queue creation.
- Suggested Fix: Retry after verifying the GPU runtime and device health.

## ATHO-MINE-108
- Category: `MINE`
- Title: GPU Kernel Create Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU runtime could not create the mining kernel instance.
- Common Cause: The compiled program does not export the expected kernel symbol.
- Suggested Fix: Verify the kernel name and compiled program contents.

## ATHO-MINE-109
- Category: `MINE`
- Title: GPU Buffer Allocation Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU runtime could not allocate required mining buffers.
- Common Cause: Device memory is exhausted or buffer sizing is invalid.
- Suggested Fix: Reduce batch size or free device memory before retrying.

## ATHO-MINE-110
- Category: `MINE`
- Title: GPU Invalid Argument
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU mining request contains invalid lengths or parameters.
- Common Cause: Header bytes, nonce offset, target bytes, or buffers are malformed.
- Suggested Fix: Rebuild the mining job from the canonical node template.

## ATHO-MINE-111
- Category: `MINE`
- Title: GPU Batch Too Large
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The requested GPU batch size exceeds the configured maximum.
- Common Cause: The batch size tuning is too aggressive for the current runtime.
- Suggested Fix: Reduce ATHO_GPU_BATCH_SIZE or increase the allowed cap deliberately.

## ATHO-MINE-112
- Category: `MINE`
- Title: GPU Nonce Overflow
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The requested nonce range would overflow the 64-bit nonce space.
- Common Cause: Start nonce and batch size extend past the maximum allowed nonce.
- Suggested Fix: Clamp the nonce range and request a fresh mining batch.

## ATHO-MINE-113
- Category: `MINE`
- Title: GPU Kernel Execution Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU runtime failed while executing the mining kernel.
- Common Cause: The device aborted execution or the runtime reported a launch failure.
- Suggested Fix: Retry after checking GPU stability and kernel compatibility.

## ATHO-MINE-114
- Category: `MINE`
- Title: GPU Buffer I/O Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU runtime failed while moving buffers to or from device memory.
- Common Cause: Driver, queue, or memory-transfer operations failed mid-job.
- Suggested Fix: Retry with a smaller batch size and verify device stability.

## ATHO-MINE-115
- Category: `MINE`
- Title: GPU Probe Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU probe could not collect structured device information.
- Common Cause: Runtime enumeration failed before a mining session started.
- Suggested Fix: Inspect the GPU runtime installation and probe logs.

## ATHO-MINE-116
- Category: `MINE`
- Title: Unknown GPU Failure
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The GPU backend returned an unknown failure.
- Common Cause: The native helper reported an unexpected or uncategorized error.
- Suggested Fix: Inspect the native helper logs and add a more specific error mapping.

## ATHO-MINE-117
- Category: `MINE`
- Title: GPU Solution Mismatch
- Severity: `critical`
- User Facing: `false`
- Consensus Critical: `true`
- Explanation: The GPU backend returned a nonce that does not match Atho's canonical CPU verification.
- Common Cause: The backend hashed different header bytes or returned a corrupted result.
- Suggested Fix: Treat the backend as faulty, reject the solution, and verify the kernel against CPU vectors.

## ATHO-LAUNCH-001
- Category: `LAUNCH`
- Title: Public RPC Bind Denied
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: Atho refuses to bind RPC to a non-loopback address without explicit opt-in.
- Common Cause: ATHO_RPC_ALLOW_PUBLIC was not set and the configured RPC bind is public.
- Suggested Fix: Use a loopback RPC address or set ATHO_RPC_ALLOW_PUBLIC=1 intentionally.

## ATHO-LAUNCH-002
- Category: `LAUNCH`
- Title: RPC Bind Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The node could not bind its RPC listener.
- Common Cause: Another process is using the port or the address is invalid.
- Suggested Fix: Choose a free RPC address and verify port permissions.

## ATHO-LAUNCH-003
- Category: `LAUNCH`
- Title: P2P Bind Failed
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The node could not bind its P2P listener.
- Common Cause: Another process is using the P2P port or the address is invalid.
- Suggested Fix: Choose a free P2P address and verify firewall or port settings.

## ATHO-LAUNCH-004
- Category: `LAUNCH`
- Title: Invalid Peer Address
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The supplied peer address is not a valid socket address.
- Common Cause: The peer string is malformed or missing a port.
- Suggested Fix: Use a valid host:port pair for manual peer configuration.

## ATHO-WALLET-001
- Category: `WALLET`
- Title: Invalid Mnemonic Word Count
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The mnemonic does not contain a supported number of words.
- Common Cause: The phrase is incomplete or uses an unsupported length.
- Suggested Fix: Use a 12, 24, or 48 word Atho mnemonic.

## ATHO-WALLET-002
- Category: `WALLET`
- Title: Invalid Mnemonic Entropy Length
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The provided entropy length does not match the requested mnemonic size.
- Common Cause: The entropy buffer is truncated or not sized for the selected mnemonic length.
- Suggested Fix: Provide entropy with the exact byte length required by the mnemonic format.

## ATHO-WALLET-003
- Category: `WALLET`
- Title: Invalid Mnemonic Word
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The mnemonic contains a word outside Atho's approved wordlist.
- Common Cause: One or more mnemonic words were mistyped or come from another wordlist.
- Suggested Fix: Correct the invalid word using the Atho mnemonic wordlist.

## ATHO-WALLET-004
- Category: `WALLET`
- Title: Mnemonic Checksum Mismatch
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The mnemonic checksum bits do not match the encoded entropy.
- Common Cause: The phrase was entered incorrectly or truncated.
- Suggested Fix: Re-enter the mnemonic exactly as generated.

## ATHO-WALLET-005
- Category: `WALLET`
- Title: Wallet File I/O Error
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: A wallet file could not be read or written.
- Common Cause: The wallet path is missing, locked, or lacks permissions.
- Suggested Fix: Verify the wallet path, file permissions, and free disk space.

## ATHO-WALLET-006
- Category: `WALLET`
- Title: Wallet Serialization Failure
- Severity: `error`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: The wallet state could not be encoded or decoded safely.
- Common Cause: The wallet file is corrupted or the serialized schema changed unexpectedly.
- Suggested Fix: Restore from a trusted backup or rebuild the wallet file.

## ATHO-WALLET-007
- Category: `WALLET`
- Title: Invalid Wallet Header
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The wallet datafile header is malformed or truncated.
- Common Cause: The wallet file is corrupt or not an Atho wallet datafile.
- Suggested Fix: Verify the file source and restore from backup if necessary.

## ATHO-WALLET-008
- Category: `WALLET`
- Title: Unsupported Wallet Version
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The wallet datafile version is not supported by this build.
- Common Cause: The wallet file was created by a different incompatible release.
- Suggested Fix: Use a compatible Atho release or migrate the wallet file.

## ATHO-WALLET-009
- Category: `WALLET`
- Title: Unsupported Wallet Encryption Mode
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The wallet uses an encryption mode that this build cannot read.
- Common Cause: The datafile header references an unknown or future encryption mode.
- Suggested Fix: Upgrade to a compatible build or re-export the wallet in a supported format.

## ATHO-WALLET-010
- Category: `WALLET`
- Title: Wallet Randomness Failure
- Severity: `fatal`
- User Facing: `false`
- Consensus Critical: `false`
- Explanation: Secure randomness could not be obtained for wallet encryption.
- Common Cause: The operating system randomness source failed or is unavailable.
- Suggested Fix: Fix the host entropy source before generating or encrypting wallet material.

## ATHO-WALLET-011
- Category: `WALLET`
- Title: Wallet Password Rejected
- Severity: `error`
- User Facing: `true`
- Consensus Critical: `false`
- Explanation: The wallet password is wrong or the encrypted wallet file is corrupted.
- Common Cause: The password is incorrect or authenticated decryption failed.
- Suggested Fix: Retry with the correct password or restore the wallet from backup.
