// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Startup dialog routing for wallet creation, import, and open flows.

mod wallet;
mod welcome;

use super::{DesktopApp, LaunchPage};
use eframe::egui;

/// Renders the currently active launch dialog.
pub(crate) fn render(app: &mut DesktopApp, ui: &mut egui::Ui) {
    match app.launch_page {
        LaunchPage::Welcome => welcome::render(app, ui),
        LaunchPage::CreateWallet => wallet::render_create(app, ui),
        LaunchPage::ImportWallet => wallet::render_import(app, ui),
        LaunchPage::OpenWallet => wallet::render_open(app, ui),
    }
}
