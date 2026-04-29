use crate::app::{mnemonic_ui, widgets, DesktopApp, LaunchPage};
use atho_wallet::mnemonic::MnemonicPhrase;
use eframe::egui;

pub(crate) fn render_create(app: &mut DesktopApp, ui: &mut egui::Ui) {
    let mut create_clicked = false;
    let mut cancel_clicked = false;

    widgets::dialog_frame().show(ui, |ui| {
        ui.set_width(700.0);
        ui.label(egui::RichText::new("Create Wallet").size(22.0).strong());
        ui.add_space(6.0);
        widgets::muted_label(ui, "Create a new Atho HD wallet. Encryption is optional.");
        ui.add_space(12.0);

        form_label(ui, "Wallet file");
        widgets::text_input(ui, &mut app.create_form.wallet_path, "");
        ui.add_space(10.0);
        ui.checkbox(
            &mut app.create_form.encrypt_wallet,
            "Encrypt wallet with passphrase",
        );
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
        ui.horizontal(|ui| {
            let ready = create_mnemonic_sentence.is_some()
                && app.create_form.acknowledged_backup
                && (!app.create_form.encrypt_wallet
                    || (!app.create_form.wallet_password.is_empty()
                        && app.create_form.wallet_password
                            == app.create_form.wallet_password_confirm));
            if ui.add_enabled(ready, egui::Button::new("Create")).clicked() {
                create_clicked = true;
            }
            if ui.button("Back").clicked() {
                cancel_clicked = true;
            }
        });
    });

    if create_clicked {
        if app.wallet_preparation_job.is_some() {
            app.last_error = Some(String::from("wallet preparation already in progress"));
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
        app.start_wallet_from_mnemonic_preparation(
            mnemonic_sentence,
            app.create_form.mnemonic_passphrase.clone(),
            app.create_form.wallet_path.clone(),
            wallet_password,
            "Preparing wallet",
        );
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

    widgets::dialog_frame().show(ui, |ui| {
        ui.set_width(700.0);
        ui.label(egui::RichText::new("Import Wallet").size(22.0).strong());
        ui.add_space(8.0);
        widgets::muted_label(
            ui,
            "Restore an Atho HD wallet from an existing recovery phrase.",
        );
        ui.add_space(16.0);

        form_label(ui, "Wallet file");
        widgets::text_input(ui, &mut app.import_form.wallet_path, "");
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
        ui.checkbox(
            &mut app.import_form.encrypt_wallet,
            "Encrypt wallet with passphrase",
        );
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
        ui.horizontal(|ui| {
            let import_mnemonic_ready =
                mnemonic_ui::mnemonic_sentence_from_words(&app.import_form.mnemonic_words).is_ok();
            let ready = (!app.import_form.encrypt_wallet
                || (!app.import_form.wallet_password.is_empty()
                    && app.import_form.wallet_password == app.import_form.wallet_password_confirm))
                && import_mnemonic_ready;
            if ui.add_enabled(ready, egui::Button::new("Import")).clicked() {
                import_clicked = true;
            }
            if ui.button("Back").clicked() {
                cancel_clicked = true;
            }
        });
    });

    if import_clicked {
        if app.wallet_preparation_job.is_some() {
            app.last_error = Some(String::from("wallet preparation already in progress"));
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
        app.start_wallet_from_mnemonic_preparation(
            mnemonic.as_sentence(),
            app.import_form.mnemonic_passphrase.clone(),
            app.import_form.wallet_path.clone(),
            wallet_password,
            "Preparing wallet",
        );
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

    widgets::dialog_frame().show(ui, |ui| {
        ui.set_width(580.0);
        ui.label(egui::RichText::new("Open Wallet").size(22.0).strong());
        ui.add_space(8.0);
        widgets::muted_label(
            ui,
            "Enter wallet passphrase if encrypted. Leave blank otherwise.",
        );
        ui.add_space(16.0);

        form_label(ui, "Wallet file");
        widgets::text_input(ui, &mut app.open_form.wallet_path, "");
        ui.add_space(8.0);
        form_label(ui, "Wallet password");
        ui.add(
            egui::TextEdit::singleline(&mut app.open_form.wallet_password)
                .desired_width(f32::INFINITY)
                .password(!app.open_form.show_password),
        );
        ui.checkbox(&mut app.open_form.show_password, "Show passphrase");
        ui.add_space(18.0);

        ui.horizontal(|ui| {
            if ui.button("Open").clicked() {
                open_clicked = true;
            }
            if ui.button("Back").clicked() {
                cancel_clicked = true;
            }
        });
    });

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
