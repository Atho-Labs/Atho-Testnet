# Atho Debug Console

The Atho Qt client now includes a registry-backed debug console similar in spirit to Bitcoin Core's debug console.

## Where To Find It

Open it from either:

- the `Help > Debug Console` menu
- the `Console` button in the top toolbar

## What It Uses

The console does **not** have its own command logic.

It uses:

- the shared command registry in [`crates/atho-rpc/src/command.rs`](/Users/eyeanonymous/Desktop/Atho-Alpha /crates/atho-rpc/src/command.rs)
- the same `ExecuteCommand` RPC path used by `atho-cli`
- the same node validation path used by the running local node

That keeps the CLI and the GUI aligned.

## Current Features

- command input
- grouped command browser
- workflow shortcuts
- registry-backed suggestions
- offline `help` rendering from the shared registry
- command group and permission labels
- output modes:
  - `Pretty`
  - `JSON`
  - `Table`
- recent-command recall buttons
- up/down history navigation
- copy-last-output
- active network display
- dangerous-command confirmation toggle
- structured error-code display through the RPC error payload

## Good Starter Commands

- `help`
- `help mining`
- `getstatus`
- `gethealth`
- `getblockchaininfo`
- `getpeerinfo`
- `gettemplateinfo`
- `validateathoaddress <address>`
- `sha3_384 ABC`

## Safety Notes

- the console does not bypass consensus
- it does not mutate chainstate directly
- the current initial command set is read/debug focused
- dangerous confirmation is already part of the UI contract for future admin commands

## Current Limitation

The debug console is only as broad as the current command registry.

That means the professional command framework is now in place, but the full Bitcoin Core-sized command surface from the long-term roadmap prompt is still follow-up work.
