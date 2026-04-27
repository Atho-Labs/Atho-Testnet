# Atho Qt Reference Map

This document maps the Bitcoin Qt reference tree in `crates/bitcoin gui referacne /` to the native Atho Rust desktop client in `crates/atho-qt/`.

The Bitcoin tree is a visual and behavioral blueprint only. The Atho client is implemented natively in Rust, with Atho-owned state, resources, and backend interfaces.

## Current Atho GUI Structure

- `crates/atho-qt/src/bin/atho-qt.rs`
- `crates/atho-qt/src/lib.rs`
- `crates/atho-qt/src/resources.rs`
- `crates/atho-qt/src/connection.rs`
- `crates/atho-qt/src/state.rs`
- `crates/atho-qt/src/view.rs`
- `crates/atho-qt/src/error.rs`
- `crates/atho-qt/src/app.rs`
- `crates/atho-qt/src/app/models.rs`
- `crates/atho-qt/src/app/shell.rs`
- `crates/atho-qt/src/app/startup.rs`
- `crates/atho-qt/src/app/theme.rs`
- `crates/atho-qt/src/app/widgets/mod.rs`
- `crates/atho-qt/src/app/dialogs/mod.rs`
- `crates/atho-qt/src/app/dialogs/welcome.rs`
- `crates/atho-qt/src/app/dialogs/wallet.rs`
- `crates/atho-qt/src/app/pages/mod.rs`
- `crates/atho-qt/src/app/pages/overview.rs`
- `crates/atho-qt/src/app/pages/send.rs`
- `crates/atho-qt/src/app/pages/receive.rs`
- `crates/atho-qt/src/app/pages/transactions.rs`
- `crates/atho-qt/src/app/pages/settings.rs`

Assets:

- `crates/atho-qt/assets/branding/atho-icon.png`
- `crates/atho-qt/assets/branding/atho-mark.png`
- `crates/atho-qt/assets/icons/*.png`

## File-by-File Modular Breakdown

| Atho file | Responsibility |
| --- | --- |
| `src/bin/atho-qt.rs` | App bootstrap, window creation, network selection, icon hookup |
| `src/lib.rs` | Public crate module surface |
| `src/app.rs` | Desktop app coordinator, wallet lifecycle, transaction/mining orchestration, background task wiring |
| `src/app/models.rs` | App-local navigation, launch flow, form state, wallet activity rows, job/result types |
| `src/app/shell.rs` | Main shell, menu bar, toolbar, status bar, about dialog |
| `src/app/startup.rs` | No-wallet startup shell |
| `src/app/theme.rs` | White theme and egui visual/style configuration |
| `src/app/widgets/mod.rs` | Shared shell/panel/button/table helpers and Atho palette constants |
| `src/app/dialogs/mod.rs` | Launch-screen routing |
| `src/app/dialogs/welcome.rs` | Welcome / no-wallet landing panel |
| `src/app/dialogs/wallet.rs` | Create / import / open wallet flows |
| `src/app/pages/mod.rs` | Active page dispatch |
| `src/app/pages/overview.rs` | Balance overview and recent activity |
| `src/app/pages/send.rs` | Send form and transaction submission UX |
| `src/app/pages/receive.rs` | Receive form and payment-request history |
| `src/app/pages/transactions.rs` | Transaction history and filtering |
| `src/app/pages/settings.rs` | Wallet / mining / client settings page |
| `src/resources.rs` | Atho asset loading and icon scaling pipeline |
| `src/connection.rs` | Backend service interface and lightweight status monitor |
| `src/state.rs` | UI state adapter for wallet snapshot and connection flags |
| `src/view.rs` | View-model adapter for network / sync presentation |
| `src/error.rs` | UI-facing error type |

## Bitcoin Reference Component Mapping

| Bitcoin reference | Atho module | Decision |
| --- | --- | --- |
| `bitcoingui.h/.cpp` | `src/app.rs`, `src/app/shell.rs`, `src/app/startup.rs` | Recreated in native Rust, preserving shell layout and navigation structure |
| `walletframe.h/.cpp` | `src/app.rs`, `src/app/pages/mod.rs`, `src/app/dialogs/*` | Adapted into a single-wallet Atho shell with startup routing |
| `walletview.h/.cpp` | `src/app/pages/mod.rs` | Adapted as Atho page dispatch rather than Qt stacked widgets |
| `overviewpage.h/.cpp` | `src/app/pages/overview.rs` | Recreated with Atho balances and recent activity |
| `sendcoinsdialog.h/.cpp` | `src/app/pages/send.rs` | Adapted for Atho send flow and background submission |
| `receivecoinsdialog.h/.cpp` | `src/app/pages/receive.rs` | Recreated for Atho receiving-address generation and request history |
| `transactionview.h/.cpp` | `src/app/pages/transactions.rs` | Recreated as a Rust table/filter view |
| `optionsdialog.h/.cpp` | `src/app/pages/settings.rs` | Adapted to Atho-specific settings and miner controls |
| `createwalletdialog.h/.cpp` | `src/app/dialogs/wallet.rs` | Recreated for Atho wallet create/import/open flows |
| `askpassphrasedialog.h/.cpp` | `src/app/dialogs/wallet.rs` | Adapted into the wallet open/create/import forms |
| `splashscreen.h/.cpp` | `src/app/startup.rs`, `src/app/dialogs/welcome.rs`, `src/resources.rs` | Recreated using the Atho logo assets |
| `modaloverlay.h/.cpp` | `src/app/startup.rs` and the wallet launch routing | Adapted as the Atho no-wallet launch path |
| `clientmodel.h/.cpp` | `src/view.rs`, `src/state.rs`, `src/connection.rs` | Replaced by Atho view-model/state adapters and backend bridge |
| `walletmodel.h/.cpp` | `crates/atho-wallet` plus Atho page adapters | Replaced by native Atho wallet code and UI snapshots |
| `walletcontroller.h/.cpp` | `src/connection.rs`, `crates/atho-node/src/system.rs` | Replaced by Atho service interfaces and RPC bridge |
| `transactiontablemodel.h/.cpp` | `src/app/models.rs`, `src/app/pages/transactions.rs` | Replaced by lightweight Rust row caching and filtering |
| `platformstyle.h/.cpp` | `src/app/theme.rs`, `src/app/widgets/mod.rs` | Replaced by a white Atho palette and egui styling |
| `networkstyle.h/.cpp` | `src/bin/atho-qt.rs`, `src/resources.rs` | Replaced by Atho branding and icon scaling |
| `guiutil.h/.cpp` | `src/app/widgets/mod.rs` and small UI helpers | Replaced by small, local Rust helpers |
| `rpcconsole.*` | not carried forward | Removed because it is not relevant to Atho |
| `signverifymessagedialog.*` | not carried forward | Removed because it is not relevant to Atho |
| `psbtoperationsdialog.*` | not carried forward | Removed because it is not relevant to Atho |
| `coincontroldialog.*` | not carried forward | Removed because Atho does not expose coin-control UX |

## Asset / Resource Replacement Plan

The Atho client uses an Atho-owned resource pipeline:

- `assets/branding/atho-icon.png` is the square app/window badge derived from the repo logo.
- `assets/branding/atho-mark.png` is the taller logo mark used for welcome and about presentations.
- `assets/icons/overview.png`, `send.png`, `receive.png`, `history.png`, `warning.png`, `export.png`, `editcopy.png`, `remove.png`, `add.png`, and `address-book.png` are renamed, Atho-owned copies of the generic UI icons used for shell tabs and common actions.
- `resources.rs` centralizes image scaling and icon loading so page modules stay free of asset-path concerns.

Bitcoin-branded assets were not carried into the Atho build:

- `bitcoin.png`
- `bitcoin.ico`
- `bitcoin.icns`
- `bitcoin_signet.ico`
- any `bitcoin`-named strings in the Atho UI code

## White Theme Adaptation Plan

The Atho UI now uses a white desktop presentation while preserving the reference structure:

- white shell background
- white panels and dialogs
- light gray borders and separators
- Atho green as the accent color
- dark text on light backgrounds
- toolbar tabs with clear selected/inactive states
- bottom status bar with compact sync presentation

The layout remains close to the Bitcoin GUI reference, but the visual treatment is Atho-branded and white rather than dark.

## Backend / UI Separation Plan

The GUI is intentionally thin:

- `connection.rs` owns the backend interface and a lightweight status monitor thread
- `app.rs` owns orchestration, not chain validation or storage logic
- `state.rs` and `view.rs` are adapters, not business-logic containers
- wallet, chain, and validation logic remain in the Atho backend crates
- send submission and status polling are handled asynchronously so the frame loop stays responsive

The UI does not own:

- blockchain validation
- wallet core logic
- chain sync logic
- mempool logic
- storage logic
- mining logic
- periodic blocking polls

## Cleanup / Removal List

Removed from the Atho client:

- Bitcoin RPC console flow
- PSBT dialogs and PSBT-specific controls
- sign/verify message UI
- coin-control dialog flow
- Bitcoin terminology in menu/status/page labels
- Bitcoin-branded icon paths
- dark Bitcoin-style theme assumptions

## Verification Notes

The current Atho Qt client is:

- native Rust
- modular
- white-themed
- Atho-branded
- backed by small UI files rather than one large GUI file
- separated from backend service interfaces
- free of Bitcoin references in the Atho UI code and resource paths

