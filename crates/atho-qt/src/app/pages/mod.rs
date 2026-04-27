mod overview;
mod receive;
mod send;
mod settings;
mod transactions;

use super::{DesktopApp, NavTab};
use eframe::egui;

pub(crate) fn render_active_page(app: &mut DesktopApp, ui: &mut egui::Ui) {
    match app.active_tab {
        NavTab::Overview => overview::render(app, ui),
        NavTab::Send => send::render(app, ui),
        NavTab::Receive => receive::render(app, ui),
        NavTab::Transactions => transactions::render(app, ui),
        NavTab::Settings => settings::render(app, ui),
    }
}
