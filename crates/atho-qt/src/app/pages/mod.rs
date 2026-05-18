// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Main application pages and routing between them.

pub(crate) mod console;
mod overview;
mod receive;
mod send;
mod settings;
mod transactions;

use super::{DesktopApp, NavTab};
use eframe::egui;

/// Dispatches rendering to the page that matches the active navigation tab.
pub(crate) fn render_active_page(app: &mut DesktopApp, ui: &mut egui::Ui) {
    match app.active_tab {
        NavTab::Overview => overview::render(app, ui),
        NavTab::Send => send::render(app, ui),
        NavTab::Receive => receive::render(app, ui),
        NavTab::Transactions => transactions::render(app, ui),
        NavTab::DebugConsole => console::render(app, ui),
        NavTab::Settings => settings::render(app, ui),
    }
}
