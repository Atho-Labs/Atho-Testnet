// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Wallet creation, import, and recovery dialogs.
use crate::app::{mnemonic_ui, widgets, DesktopApp, LaunchPage, MnemonicWalletPreparationRequest};
use atho_wallet::mnemonic::MnemonicPhrase;
use eframe::egui;
use rfd::FileDialog;

pub(crate) fn render_create(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let mut create_clicked = false;
    let mut cancel_clicked = false;
    let mut wallet_folder_browse_clicked = false;

    widgets::dialog_frame().show(ui, |ui| {
        let card_width = ui.available_width().clamp(320.0, 700.0);
        ui.set_width(card_width);
        ui.set_max_width(card_width);
        ui.label(egui::RichText::new("Create Wallet").size(22.0).strong());
        ui.add_space(6.0);
        widgets::muted_label(ui, "Create a new Atho HD wallet. Encryption is optional.");
        ui.add_space(12.0);

        form_label(ui, "Wallet name");
        widgets::text_input(ui, &mut app.create_form.wallet_name, "Wallet 1");
        ui.add_space(10.0);
        form_label(ui, "Wallet location");
        render_browse_row(
            ui,
            &mut app.create_form.wallet_path,
            "Choose wallet folder",
            &mut wallet_folder_browse_clicked,
        );
        widgets::muted_label(
            ui,
            "Each wallet is stored in its own folder and uses a local .datafile inside it.",
        );
        ui.add_space(10.0);
        let wallet_encryption_required = app.node_settings_form.wallet_require_encryption;
        if wallet_encryption_required {
            app.create_form.encrypt_wallet = true;
        }
        ui.add_enabled(
            !wallet_encryption_required,
            egui::Checkbox::new(
                &mut app.create_form.encrypt_wallet,
                "Encrypt wallet with passphrase",
            ),
        );
        if wallet_encryption_required {
            widgets::muted_label(ui, "Required by Node Settings.");
        }
        if app.create_form.encrypt_wallet {
            ui.add_space(8.0);
            form_label(ui, "Wallet passphrase");
            ui.add(
                egui::TextEdit::singleline(&mut app.create_form.wallet_password)
                    .desired_width(f32::INFINITY)
                    .password(!app.create_form.show_passwords),
            );
            ui.add_space(8.0);
            form_label(ui, "Confirm passphrase");
            ui.add(
                egui::TextEdit::singleline(&mut app.create_form.wallet_password_confirm)
                    .desired_width(f32::INFINITY)
                    .password(!app.create_form.show_passwords),
            );
        } else {
            app.create_form.wallet_password.clear();
            app.create_form.wallet_password_confirm.clear();
        }
        ui.add_space(8.0);
        form_label(ui, "Seed passphrase (optional)");
        ui.add(
            egui::TextEdit::singleline(&mut app.create_form.mnemonic_passphrase)
                .desired_width(f32::INFINITY)
                .password(!app.create_form.show_passwords),
        );
        ui.checkbox(&mut app.create_form.show_passwords, "Show passphrases");
        ui.add_space(14.0);

        let mut selected_word_count = app.create_form.mnemonic_word_count;
        if let Some(changed_to) =
            mnemonic_ui::render_word_count_picker(ui, &mut selected_word_count)
        {
            app.create_form.set_mnemonic_word_count(changed_to);
            if let Err(err) = app.generate_create_mnemonic() {
                app.last_error = Some(err);
            }
        }
        ui.add_space(8.0);

        let create_mnemonic_sentence =
            mnemonic_ui::mnemonic_sentence_from_words(&app.create_form.mnemonic_words).ok();
        if create_mnemonic_sentence.is_some() {
            ui.colored_label(
                widgets::ACCENT,
                "Write this recovery phrase down now. It is shown once.",
            );
            ui.add_space(8.0);
            mnemonic_ui::render_word_grid(
                ui,
                &mut app.create_form.mnemonic_words,
                false,
                "create_mnemonic_grid",
                false,
            );
            ui.horizontal(|ui| {
                if ui.button("Copy phrase").clicked() {
                    DesktopApp::copy_text(ui, create_mnemonic_sentence.clone().unwrap_or_default());
                }
                ui.checkbox(
                    &mut app.create_form.acknowledged_backup,
                    "I have backed up the recovery phrase",
                );
            });
        }

        ui.add_space(18.0);
        ui.horizontal_wrapped(|ui| {
            let wallet_password_ready = !app.create_form.encrypt_wallet
                || (!app.create_form.wallet_password.is_empty()
                    && app.create_form.wallet_password == app.create_form.wallet_password_confirm);
            let ready = create_mnemonic_sentence.is_some()
                && app.create_form.acknowledged_backup
                && wallet_password_ready;
            if ui.add_enabled(ready, egui::Button::new("Create")).clicked() {
                create_clicked = true;
            }
            if ui.button("Back").clicked() {
                cancel_clicked = true;
            }
        });
    });

    if wallet_folder_browse_clicked {
        if let Some(path) = pick_wallet_folder() {
            app.create_form.wallet_path = path;
        }
    }

    if create_clicked {
        if app.wallet_preparation_job.is_some() {
            app.last_error = Some(String::from("wallet preparation already in progress"));
            return;
        }

        if app.create_form.wallet_name.trim().is_empty() {
            app.last_error = Some(String::from("Enter a wallet name"));
            return;
        }

        if app.node_settings_form.wallet_require_encryption && !app.create_form.encrypt_wallet {
            app.last_error = Some(String::from(
                "Wallet passphrase is required by Node Settings",
            ));
            return;
        }

        if app.create_form.encrypt_wallet
            && app.create_form.wallet_password != app.create_form.wallet_password_confirm
        {
            app.last_error = Some(String::from("wallet passwords do not match"));
            return;
        }

        let mnemonic_sentence =
            match mnemonic_ui::mnemonic_sentence_from_words(&app.create_form.mnemonic_words) {
                Ok(sentence) => sentence,
                Err(err) => {
                    app.last_error = Some(err);
                    return;
                }
            };

        if let Err(err) = MnemonicPhrase::parse(&mnemonic_sentence) {
            app.last_error = Some(err.to_string());
            return;
        }

        let wallet_password = if app.create_form.encrypt_wallet {
            app.create_form.wallet_password.clone()
        } else {
            String::new()
        };
        app.start_wallet_from_mnemonic_preparation(MnemonicWalletPreparationRequest {
            mnemonic_text: mnemonic_sentence,
            mnemonic_passphrase: app.create_form.mnemonic_passphrase.clone(),
            wallet_path: app.create_form.wallet_path.clone(),
            wallet_password,
            wallet_name: app.create_form.wallet_name.trim().to_owned(),
            wallet_word_count: app.create_form.mnemonic_word_count,
            stage: "Preparing wallet",
        });
        app.create_form.wallet_password.clear();
        app.create_form.wallet_password_confirm.clear();
    }

    if cancel_clicked {
        app.create_form.reset_phrase();
        app.launch_page = LaunchPage::Welcome;
    }
}

pub(crate) fn render_import(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let mut import_clicked = false;
    let mut cancel_clicked = false;
    let mut wallet_folder_browse_clicked = false;

    widgets::dialog_frame().show(ui, |ui| {
        let card_width = ui.available_width().clamp(320.0, 700.0);
        ui.set_width(card_width);
        ui.set_max_width(card_width);
        ui.label(egui::RichText::new("Import Wallet").size(22.0).strong());
        ui.add_space(8.0);
        widgets::muted_label(
            ui,
            "Restore an Atho HD wallet from an existing recovery phrase.",
        );
        ui.add_space(16.0);

        form_label(ui, "Wallet name");
        widgets::text_input(ui, &mut app.import_form.wallet_name, "Wallet 1");
        ui.add_space(8.0);
        form_label(ui, "Wallet location");
        render_browse_row(
            ui,
            &mut app.import_form.wallet_path,
            "Choose wallet folder",
            &mut wallet_folder_browse_clicked,
        );
        ui.add_space(8.0);
        form_label(ui, "Mnemonic phrase");
        let mut selected_word_count = app.import_form.mnemonic_word_count;
        if let Some(changed_to) =
            mnemonic_ui::render_word_count_picker(ui, &mut selected_word_count)
        {
            app.import_form.set_mnemonic_word_count(changed_to);
        }
        ui.add_space(8.0);
        mnemonic_ui::render_word_grid(
            ui,
            &mut app.import_form.mnemonic_words,
            true,
            "import_mnemonic_grid",
            true,
        );
        ui.add_space(8.0);
        form_label(ui, "Seed passphrase (optional)");
        widgets::text_input(ui, &mut app.import_form.mnemonic_passphrase, "");
        ui.add_space(8.0);
        let wallet_encryption_required = app.node_settings_form.wallet_require_encryption;
        if wallet_encryption_required {
            app.import_form.encrypt_wallet = true;
        }
        ui.add_enabled(
            !wallet_encryption_required,
            egui::Checkbox::new(
                &mut app.import_form.encrypt_wallet,
                "Encrypt wallet with passphrase",
            ),
        );
        if wallet_encryption_required {
            widgets::muted_label(ui, "Required by Node Settings.");
        }
        if app.import_form.encrypt_wallet {
            ui.add_space(8.0);
            form_label(ui, "Wallet passphrase");
            ui.add(
                egui::TextEdit::singleline(&mut app.import_form.wallet_password)
                    .desired_width(f32::INFINITY)
                    .password(!app.import_form.show_passwords),
            );
            ui.add_space(8.0);
            form_label(ui, "Confirm passphrase");
            ui.add(
                egui::TextEdit::singleline(&mut app.import_form.wallet_password_confirm)
                    .desired_width(f32::INFINITY)
                    .password(!app.import_form.show_passwords),
            );
        } else {
            app.import_form.wallet_password.clear();
            app.import_form.wallet_password_confirm.clear();
        }
        ui.checkbox(&mut app.import_form.show_passwords, "Show passphrases");
        ui.add_space(18.0);
        ui.horizontal_wrapped(|ui| {
            let import_mnemonic_ready =
                mnemonic_ui::mnemonic_sentence_from_words(&app.import_form.mnemonic_words).is_ok();
            let wallet_password_ready = !app.import_form.encrypt_wallet
                || (!app.import_form.wallet_password.is_empty()
                    && app.import_form.wallet_password == app.import_form.wallet_password_confirm);
            let ready = wallet_password_ready && import_mnemonic_ready;
            if ui.add_enabled(ready, egui::Button::new("Import")).clicked() {
                import_clicked = true;
            }
            if ui.button("Back").clicked() {
                cancel_clicked = true;
            }
        });
    });

    if wallet_folder_browse_clicked {
        if let Some(path) = pick_wallet_folder() {
            app.import_form.wallet_path = path;
        }
    }

    if import_clicked {
        if app.wallet_preparation_job.is_some() {
            app.last_error = Some(String::from("wallet preparation already in progress"));
            return;
        }

        if app.import_form.wallet_name.trim().is_empty() {
            app.last_error = Some(String::from("Enter a wallet name"));
            return;
        }

        if app.node_settings_form.wallet_require_encryption && !app.import_form.encrypt_wallet {
            app.last_error = Some(String::from(
                "Wallet passphrase is required by Node Settings",
            ));
            return;
        }

        if app.import_form.encrypt_wallet
            && app.import_form.wallet_password != app.import_form.wallet_password_confirm
        {
            app.last_error = Some(String::from("wallet passwords do not match"));
            return;
        }
        let mnemonic_sentence =
            match mnemonic_ui::mnemonic_sentence_from_words(&app.import_form.mnemonic_words) {
                Ok(sentence) => sentence,
                Err(err) => {
                    app.last_error = Some(err);
                    return;
                }
            };
        let mnemonic = match MnemonicPhrase::parse(&mnemonic_sentence) {
            Ok(mnemonic) => mnemonic,
            Err(err) => {
                app.last_error = Some(err.to_string());
                return;
            }
        };
        let wallet_password = if app.import_form.encrypt_wallet {
            app.import_form.wallet_password.clone()
        } else {
            String::new()
        };
        app.start_wallet_from_mnemonic_preparation(MnemonicWalletPreparationRequest {
            mnemonic_text: mnemonic.as_sentence(),
            mnemonic_passphrase: app.import_form.mnemonic_passphrase.clone(),
            wallet_path: app.import_form.wallet_path.clone(),
            wallet_password,
            wallet_name: app.import_form.wallet_name.trim().to_owned(),
            wallet_word_count: app.import_form.mnemonic_word_count,
            stage: "Preparing wallet",
        });
        app.import_form.wallet_password.clear();
        app.import_form.wallet_password_confirm.clear();
    }

    if cancel_clicked {
        app.launch_page = LaunchPage::Welcome;
    }
}

pub(crate) fn render_open(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let mut open_clicked = false;
    let mut cancel_clicked = false;
    let mut wallet_folder_browse_clicked = false;

    widgets::dialog_frame().show(ui, |ui| {
        let card_width = ui.available_width().clamp(320.0, 580.0);
        ui.set_width(card_width);
        ui.set_max_width(card_width);
        ui.label(egui::RichText::new("Open Wallet").size(22.0).strong());
        ui.add_space(8.0);
        widgets::muted_label(
            ui,
            "Enter wallet passphrase if encrypted. Leave blank otherwise.",
        );
        ui.add_space(16.0);

        form_label(ui, "Wallet folder");
        render_browse_row(
            ui,
            &mut app.open_form.wallet_path,
            "Choose wallet folder",
            &mut wallet_folder_browse_clicked,
        );
        widgets::muted_label(
            ui,
            "Select the wallet folder. The wallet datafile inside it is opened automatically.",
        );
        ui.add_space(8.0);
        form_label(ui, "Wallet password");
        ui.add(
            egui::TextEdit::singleline(&mut app.open_form.wallet_password)
                .desired_width(f32::INFINITY)
                .password(!app.open_form.show_password),
        );
        ui.checkbox(&mut app.open_form.show_password, "Show passphrase");
        ui.add_space(18.0);

        ui.horizontal_wrapped(|ui| {
            if ui.button("Open").clicked() {
                open_clicked = true;
            }
            if ui.button("Back").clicked() {
                cancel_clicked = true;
            }
        });
    });

    if wallet_folder_browse_clicked {
        if let Some(path) = pick_wallet_folder() {
            app.open_form.wallet_path = path;
        }
    }

    if open_clicked {
        if app.wallet_preparation_job.is_some() {
            app.last_error = Some(String::from("wallet preparation already in progress"));
            return;
        }
        app.start_open_wallet_preparation(
            app.open_form.wallet_path.clone(),
            app.open_form.wallet_password.clone(),
        );
        app.open_form.wallet_password.clear();
    }

    if cancel_clicked {
        app.launch_page = LaunchPage::Welcome;
    }
}

fn form_label(ui: &mut egui::Ui, label: &str) {
    ui.label(egui::RichText::new(label).size(16.0).strong());
}

fn render_browse_row(ui: &mut egui::Ui, text: &mut String, button_label: &str, clicked: &mut bool) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(text)
                .desired_width((ui.available_width() - 110.0).max(140.0))
                .hint_text("Select a wallet path"),
        );
        if ui
            .add_sized([96.0, 28.0], egui::Button::new(button_label))
            .clicked()
        {
            *clicked = true;
        }
    });
}

fn pick_wallet_folder() -> Option<String> {
    FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned())
}
