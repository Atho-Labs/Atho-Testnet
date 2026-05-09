//! Shared mnemonic and recovery-phrase UI widgets.
use super::widgets;
use atho_wallet::mnemonic::{normalize_mnemonic_words, DEFAULT_MNEMONIC_WORD_COUNT};
use eframe::egui;

pub(crate) const SUPPORTED_MNEMONIC_WORD_COUNTS: [usize; 3] = [12, 24, 48];

pub(crate) fn empty_words(count: usize) -> Vec<String> {
    vec![String::new(); count.max(1)]
}

pub(crate) fn words_from_sentence(sentence: &str) -> Vec<String> {
    let words = normalize_mnemonic_words(sentence);
    if words.is_empty() {
        empty_words(DEFAULT_MNEMONIC_WORD_COUNT)
    } else {
        words
    }
}

pub(crate) fn mnemonic_sentence_from_words(words: &[String]) -> Result<String, String> {
    let normalized = words
        .iter()
        .map(|word| normalize_word(word))
        .collect::<Vec<_>>();
    let filled = normalized.iter().filter(|word| !word.is_empty()).count();
    if filled == 0 {
        return Err(String::from("Mnemonic phrase is required"));
    }

    let missing_positions = normalized
        .iter()
        .enumerate()
        .filter_map(|(index, word)| word.is_empty().then_some(index + 1))
        .collect::<Vec<_>>();
    if !missing_positions.is_empty() {
        return Err(format!(
            "Mnemonic is missing word(s) at position(s): {}",
            missing_positions
                .iter()
                .map(|position| position.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    Ok(normalized.join(" "))
}

pub(crate) fn render_word_count_picker(
    ui: &mut egui::Ui,
    selected_count: &mut usize,
) -> Option<usize> {
    let mut changed_to = None;
    ui.horizontal_wrapped(|ui| {
        widgets::muted_label(ui, "Word count");
        for count in SUPPORTED_MNEMONIC_WORD_COUNTS {
            let selected = *selected_count == count;
            let label = format!("{count} words");
            if widgets::compact_tab(ui, selected, &label, 92.0).clicked() {
                *selected_count = count;
                changed_to = Some(count);
            }
        }
    });
    changed_to
}

pub(crate) fn render_word_grid(
    ui: &mut egui::Ui,
    words: &mut Vec<String>,
    editable: bool,
    id_source: &str,
    auto_resize_on_paste: bool,
) {
    let columns = mnemonic_grid_columns(ui.available_width(), words.len());
    let spacing = (columns.saturating_sub(1)) as f32 * 8.0;
    let card_width = ((ui.available_width() - spacing) / columns as f32).clamp(92.0, 160.0);

    egui::Grid::new(id_source)
        .num_columns(columns)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for index in 0..words.len() {
                let mut maybe_distribution = None;
                egui::Frame::none()
                    .fill(widgets::SHELL_BG)
                    .stroke(egui::Stroke::new(1.0, widgets::PANEL_STROKE))
                    .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                    .show(ui, |ui| {
                        ui.set_width(card_width);
                        ui.label(
                            egui::RichText::new(format!("{:02}", index + 1))
                                .size(10.5)
                                .monospace()
                                .color(widgets::MUTED),
                        );
                        ui.add_space(2.0);
                        if editable {
                            let response = ui.add_sized(
                                [card_width - 16.0, 28.0],
                                egui::TextEdit::singleline(&mut words[index])
                                    .hint_text("word")
                                    .desired_width(f32::INFINITY),
                            );
                            if response.changed() {
                                maybe_distribution =
                                    Some((index, normalized_tokens_from_input(&words[index])));
                            }
                        } else {
                            ui.label(
                                egui::RichText::new(words[index].as_str())
                                    .size(13.0)
                                    .strong()
                                    .color(widgets::TEXT),
                            );
                        }
                    });

                if let Some((start_index, tokens)) = maybe_distribution {
                    if tokens.len() <= 1 {
                        words[start_index] = tokens.into_iter().next().unwrap_or_default();
                    } else {
                        if auto_resize_on_paste
                            && start_index == 0
                            && SUPPORTED_MNEMONIC_WORD_COUNTS.contains(&tokens.len())
                        {
                            words.resize_with(tokens.len(), String::new);
                            words.truncate(tokens.len());
                        }
                        for (offset, token) in tokens.into_iter().enumerate() {
                            let target = start_index + offset;
                            if target >= words.len() {
                                break;
                            }
                            words[target] = token;
                        }
                    }
                }

                if (index + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
}

fn normalized_tokens_from_input(input: &str) -> Vec<String> {
    normalize_mnemonic_words(input)
}

fn normalize_word(word: &str) -> String {
    normalize_mnemonic_words(word)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn mnemonic_grid_columns(available_width: f32, word_count: usize) -> usize {
    let preferred = if word_count > 24 { 6 } else { 4 }.min(word_count.max(1));
    let mut columns = preferred.max(1);
    while columns > 1 {
        let spacing = (columns.saturating_sub(1)) as f32 * 8.0;
        let per_column = (available_width - spacing) / columns as f32;
        if per_column >= 118.0 {
            break;
        }
        columns -= 1;
    }
    columns.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonic_sentence_reports_missing_positions() {
        let mut words = empty_words(4);
        words[0] = String::from("alpha");
        words[2] = String::from("gamma");
        let error = mnemonic_sentence_from_words(&words).expect_err("missing positions");
        assert!(error.contains("2"));
        assert!(error.contains("4"));
    }

    #[test]
    fn words_from_sentence_normalizes_spacing() {
        let words = words_from_sentence("  Alpha   beta\nGAMMA  ");
        assert_eq!(words, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn mnemonic_grid_columns_collapse_on_narrow_widths() {
        assert_eq!(mnemonic_grid_columns(720.0, 12), 4);
        assert_eq!(mnemonic_grid_columns(360.0, 12), 2);
        assert_eq!(mnemonic_grid_columns(220.0, 12), 1);
        assert_eq!(mnemonic_grid_columns(900.0, 48), 6);
    }
}
