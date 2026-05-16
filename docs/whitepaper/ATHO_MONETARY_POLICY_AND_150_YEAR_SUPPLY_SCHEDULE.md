# Atho Monetary Policy and 150-Year Supply Schedule

**Ghost Genull**  
**Contact:** labs@atho.io  
**The Platinum Standard of the Quantum Age**

## 1. Executive Summary

This attachment summarizes the monetary behavior enforced by the current Atho codebase in this repository. The active implementation uses:

- a **100-second** target block time
- **12 decimals**
- **1 ATHO = 1,000,000,000,000 atoms**
- an initial block reward of **5 ATHO**
- a halving interval of **1,260,000 blocks**
- a perpetual tail reward of **0.625 ATHO**
- **no finite maximum ATHO supply cap** in current consensus

Full nodes enforce block subsidy correctness, fee accounting, ownership validation, and spendability rules independently. The schedule below summarizes subsidy issuance over 150 years under the code as written today.

## 2. Network Monetary Parameters

| Parameter | Current Code Value |
|---|---|
| Target block time | 100 seconds |
| Blocks per hour | 36 |
| Blocks per day | 864 |
| Blocks per year | 315,360 |
| Decimals | 12 |
| Atoms per ATHO | 1,000,000,000,000 |
| Initial block reward | 5 ATHO |
| Halving interval | 1,260,000 blocks |
| Tail reward | 0.625 ATHO |
| Tail emission start | Block 3,780,000 |
| Coinbase maturity | 100 blocks |
| Standard confirmations | 6 across all networks |
| Proof-of-Work hash | SHA3-384 |
| Signature scheme | Falcon-512 |
| Emission type | Halving schedule with perpetual tail emission |
| Supply enforcement rule | Per-block subsidy validation by full nodes |
| Fixed max supply cap | None in the current code |

## 3. Emission Formula

The live repository implements the following reward rule:

```text
reward(height) = max(5 / 2^floor(height / 1,260,000), 0.625) ATHO
```

In plain language:

1. Subsidy begins at 5 ATHO.
2. It halves every 1,260,000 blocks.
3. Once the halved value would drop below 0.625 ATHO, the reward floors at 0.625 ATHO permanently.
4. No finite cap currently stops emission after tail activation.

## 4. 150-Year Supply Schedule

Table 1  
*Atho 150-Year Monetary Supply Projection Under the Current Emission Policy*

| Year | Estimated Block Height | Block Reward (ATHO) | Annual Emission (ATHO) | Cumulative Supply (ATHO) | % of Enforced Max Supply | Remaining to Enforced Max Supply |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 315,360 | 5 | 1,576,800 | 1,576,800 | N/A | N/A |
| 2 | 630,720 | 5 | 1,576,800 | 3,153,600 | N/A | N/A |
| 3 | 946,080 | 5 | 1,576,800 | 4,730,400 | N/A | N/A |
| 4 | 1,261,440 | 2.5 | 1,573,200 | 6,303,600 | N/A | N/A |
| 5 | 1,576,800 | 2.5 | 788,400 | 7,092,000 | N/A | N/A |
| 10 | 3,153,600 | 1.25 | 394,200 | 10,242,000 | N/A | N/A |
| 15 | 4,730,400 | 0.625 | 197,100 | 11,619,000 | N/A | N/A |
| 20 | 6,307,200 | 0.625 | 197,100 | 12,604,500 | N/A | N/A |
| 25 | 7,884,000 | 0.625 | 197,100 | 13,590,000 | N/A | N/A |
| 30 | 9,460,800 | 0.625 | 197,100 | 14,575,500 | N/A | N/A |
| 40 | 12,614,400 | 0.625 | 197,100 | 16,546,500 | N/A | N/A |
| 50 | 15,768,000 | 0.625 | 197,100 | 18,517,500 | N/A | N/A |
| 75 | 23,652,000 | 0.625 | 197,100 | 23,445,000 | N/A | N/A |
| 100 | 31,536,000 | 0.625 | 197,100 | 28,372,500 | N/A | N/A |
| 125 | 39,420,000 | 0.625 | 197,100 | 33,300,000 | N/A | N/A |
| 150 | 47,304,000 | 0.625 | 197,100 | 38,227,500 | N/A | N/A |

Note. Projection assumes the current code constants remain unchanged for the entire horizon. It uses 100-second blocks, 315,360 blocks per year, and subsidy issuance only. Because the current code does not enforce a finite maximum ATHO supply, cap-based percentage and remaining-supply fields are not applicable.

## 5. Notes and Assumptions

1. The projection is based on the active code, not on older planning text.
2. Yearly checkpoints represent end-of-year heights under the fixed target cadence.
3. Block timestamps in practice may vary, but the subsidy function is height-based.
4. Transaction fees are excluded because they do not mint new ATHO.
5. Tail emission begins at block 3,780,000, or roughly year 11.99 under current constants.
6. The current repository does not enforce a 78,000,000 ATHO maximum supply.

## 6. Final Policy Statement

Atho's monetary policy is designed to be deterministic, auditable, and enforceable by every full node. Under the current code, no block may create ATHO beyond the consensus-defined subsidy schedule for its height, but the present implementation does not yet enforce a finite maximum ATHO supply cap.

## 7. Founder-Review Mismatch Note

The current code now matches the 100-second block cadence, 6-confirmation policy, 100-block coinbase maturity, and revised 5 -> 2.5 -> 1.25 -> 0.625 subsidy path described in updated monetary planning materials. Any public statement claiming a fixed 78,000,000 ATHO cap would still be inaccurate unless the code changes to enforce one.
