// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use atho_core::constants::ATOMS_PER_ATHO;
use serde::{Deserialize, Serialize};

pub(crate) const ATOMS_PER_MILLIATHO: u64 = 1_000_000_000;
pub(crate) const ATOMS_PER_MICROATHO: u64 = 1_000_000;
pub(crate) const ATOMS_PER_NANOATHO: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum DisplayUnit {
    #[default]
    Auto,
    Atho,
    MilliAtho,
    MicroAtho,
    NanoAtho,
    Atom,
}

impl DisplayUnit {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Atho => "ATHO",
            Self::MilliAtho => "mATHO",
            Self::MicroAtho => "μATHO",
            Self::NanoAtho => "nATHO",
            Self::Atom => "atoms",
        }
    }

    pub(crate) fn variants() -> [DisplayUnit; 6] {
        [
            Self::Auto,
            Self::Atho,
            Self::MilliAtho,
            Self::MicroAtho,
            Self::NanoAtho,
            Self::Atom,
        ]
    }

    pub(crate) fn effective_for_amount(self, amount_atoms: u64) -> DisplayUnit {
        match self {
            Self::Auto => {
                if amount_atoms >= ATOMS_PER_ATHO {
                    Self::Atho
                } else if amount_atoms >= ATOMS_PER_MILLIATHO {
                    Self::MilliAtho
                } else if amount_atoms >= ATOMS_PER_MICROATHO {
                    Self::MicroAtho
                } else if amount_atoms >= ATOMS_PER_NANOATHO {
                    Self::NanoAtho
                } else {
                    Self::Atom
                }
            }
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum InputUnit {
    #[default]
    Atho,
    MilliAtho,
    MicroAtho,
    NanoAtho,
    Atom,
}

impl InputUnit {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Atho => "ATHO",
            Self::MilliAtho => "mATHO",
            Self::MicroAtho => "μATHO",
            Self::NanoAtho => "nATHO",
            Self::Atom => "atoms",
        }
    }

    pub(crate) fn variants() -> [InputUnit; 5] {
        [
            Self::Atho,
            Self::MilliAtho,
            Self::MicroAtho,
            Self::NanoAtho,
            Self::Atom,
        ]
    }

    pub(crate) fn factor(self) -> u64 {
        match self {
            Self::Atho => ATOMS_PER_ATHO,
            Self::MilliAtho => ATOMS_PER_MILLIATHO,
            Self::MicroAtho => ATOMS_PER_MICROATHO,
            Self::NanoAtho => ATOMS_PER_NANOATHO,
            Self::Atom => 1,
        }
    }

    pub(crate) fn max_decimals(self) -> usize {
        match self {
            Self::Atho => 12,
            Self::MilliAtho => 9,
            Self::MicroAtho => 6,
            Self::NanoAtho => 3,
            Self::Atom => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct ClientDisplayPreferences {
    pub(crate) display_unit: DisplayUnit,
    pub(crate) send_input_unit: InputUnit,
}

impl ClientDisplayPreferences {
    pub(crate) fn with_display_unit(mut self, unit: DisplayUnit) -> Self {
        self.display_unit = unit;
        self
    }

    pub(crate) fn with_send_input_unit(mut self, unit: InputUnit) -> Self {
        self.send_input_unit = unit;
        self
    }
}

pub(crate) fn format_amount_atoms(amount_atoms: u64, display_unit: DisplayUnit) -> String {
    let effective = display_unit.effective_for_amount(amount_atoms);
    format_amount_in_fixed_unit(amount_atoms, effective)
}

pub(crate) fn format_amount_atoms_without_unit(amount_atoms: u64, input_unit: InputUnit) -> String {
    format_amount_components(
        amount_atoms,
        input_unit.factor(),
        input_unit.max_decimals(),
        true,
    )
}

pub(crate) fn format_fee_atoms(amount_atoms: u64, display_unit: DisplayUnit) -> String {
    if matches!(display_unit, DisplayUnit::Atom)
        || matches!(
            display_unit.effective_for_amount(amount_atoms),
            DisplayUnit::Atom
        )
    {
        format!("{} atoms", amount_atoms)
    } else {
        format!(
            "{} ({})",
            format_amount_atoms(amount_atoms, DisplayUnit::Atom),
            format_amount_atoms(amount_atoms, display_unit)
        )
    }
}

pub(crate) fn parse_amount_to_atoms(input: &str, input_unit: InputUnit) -> Result<u64, String> {
    let normalized: String = input
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != ',')
        .collect();
    if normalized.is_empty() {
        return Err(String::from("Enter an amount"));
    }
    if normalized.starts_with('-') {
        return Err(String::from("Amount must be greater than zero"));
    }
    let normalized = normalized.strip_prefix('+').unwrap_or(&normalized);
    let mut parts = normalized.split('.');
    let whole_text = parts.next().unwrap_or_default();
    let fractional_text = parts.next();
    if parts.next().is_some() {
        return Err(String::from("Amount may contain only one decimal point"));
    }
    if !whole_text.is_empty() && !whole_text.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(String::from(
            "Amount must contain only digits, commas, and one decimal point",
        ));
    }

    let whole_units = if whole_text.is_empty() {
        0
    } else {
        whole_text
            .parse::<u64>()
            .map_err(|_| String::from("Amount is too large"))?
    };

    let max_decimals = input_unit.max_decimals();
    let fractional_units = match fractional_text {
        None | Some("") => 0,
        Some(text) => {
            if max_decimals == 0 {
                return Err(String::from("atoms cannot be fractional"));
            }
            if text.len() > max_decimals {
                return Err(format!(
                    "Amount supports up to {max_decimals} decimal places"
                ));
            }
            if !text.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(String::from(
                    "Amount must contain only digits, commas, and one decimal point",
                ));
            }
            let mut padded = text.to_string();
            while padded.len() < max_decimals {
                padded.push('0');
            }
            padded
                .parse::<u64>()
                .map_err(|_| String::from("Amount is too large"))?
        }
    };

    let atoms = whole_units
        .checked_mul(input_unit.factor())
        .and_then(|value| value.checked_add(fractional_units))
        .ok_or_else(|| String::from("Amount is too large"))?;
    if atoms == 0 {
        return Err(String::from("Amount must be greater than zero"));
    }
    Ok(atoms)
}

fn format_amount_in_fixed_unit(amount_atoms: u64, unit: DisplayUnit) -> String {
    match unit {
        DisplayUnit::Atho => format!(
            "{} ATHO",
            format_amount_components(amount_atoms, ATOMS_PER_ATHO, 12, true)
        ),
        DisplayUnit::MilliAtho => format!(
            "{} mATHO",
            format_amount_components(amount_atoms, ATOMS_PER_MILLIATHO, 9, false)
        ),
        DisplayUnit::MicroAtho => format!(
            "{} μATHO",
            format_amount_components(amount_atoms, ATOMS_PER_MICROATHO, 6, false)
        ),
        DisplayUnit::NanoAtho => format!(
            "{} nATHO",
            format_amount_components(amount_atoms, ATOMS_PER_NANOATHO, 3, false)
        ),
        DisplayUnit::Atom | DisplayUnit::Auto => format_atom_label(amount_atoms),
    }
}

fn format_amount_components(
    amount_atoms: u64,
    unit_factor: u64,
    decimals: usize,
    trim_fractional: bool,
) -> String {
    if decimals == 0 {
        return format_grouped_u64(amount_atoms);
    }
    let whole = amount_atoms / unit_factor;
    let fractional = amount_atoms % unit_factor;
    if fractional == 0 {
        return format_grouped_u64(whole);
    }
    let mut fractional_text = format!("{fractional:0decimals$}");
    if trim_fractional {
        while fractional_text.ends_with('0') {
            fractional_text.pop();
        }
    }
    format!("{}.{}", format_grouped_u64(whole), fractional_text)
}

fn format_atom_label(amount_atoms: u64) -> String {
    if amount_atoms == 1 {
        String::from("1 atom")
    } else {
        format!("{} atoms", format_grouped_u64(amount_atoms))
    }
}

fn format_grouped_u64(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let remainder = digits.len() % 3;
    for (index, ch) in digits.chars().enumerate() {
        if index != 0
            && (index == remainder || (index > remainder && (index - remainder).is_multiple_of(3)))
        {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_unit_boundaries() {
        assert_eq!(
            format_amount_atoms(ATOMS_PER_ATHO, DisplayUnit::Atho),
            "1 ATHO"
        );
        assert_eq!(
            format_amount_atoms(ATOMS_PER_MILLIATHO, DisplayUnit::MilliAtho),
            "1 mATHO"
        );
        assert_eq!(
            format_amount_atoms(ATOMS_PER_MICROATHO, DisplayUnit::MicroAtho),
            "1 μATHO"
        );
        assert_eq!(
            format_amount_atoms(ATOMS_PER_NANOATHO, DisplayUnit::NanoAtho),
            "1 nATHO"
        );
        assert_eq!(format_amount_atoms(1, DisplayUnit::Atom), "1 atom");
        assert_eq!(format_amount_atoms(650, DisplayUnit::Atom), "650 atoms");
        assert_eq!(
            format_amount_atoms(650, DisplayUnit::NanoAtho),
            "0.650 nATHO"
        );
        assert_eq!(
            format_amount_atoms(625_000_000_000, DisplayUnit::Atho),
            "0.625 ATHO"
        );
        assert_eq!(
            format_amount_atoms(5_000_000_000_000, DisplayUnit::Atho),
            "5 ATHO"
        );
        assert_eq!(
            format_amount_atoms(12_458_300_000_000, DisplayUnit::MicroAtho),
            "12,458,300 μATHO"
        );
        assert_eq!(
            format_amount_atoms(12_458_300_000_000, DisplayUnit::Atom),
            "12,458,300,000,000 atoms"
        );
    }

    #[test]
    fn auto_display_selects_expected_ladder_step() {
        assert_eq!(
            format_amount_atoms(1_500_000_000_000, DisplayUnit::Auto),
            "1.5 ATHO"
        );
        assert_eq!(
            format_amount_atoms(5_000_000_000, DisplayUnit::Auto),
            "5 mATHO"
        );
        assert_eq!(
            format_amount_atoms(25_000_000, DisplayUnit::Auto),
            "25 μATHO"
        );
        assert_eq!(format_amount_atoms(50_000, DisplayUnit::Auto), "50 nATHO");
        assert_eq!(format_amount_atoms(650, DisplayUnit::Auto), "650 atoms");
    }

    #[test]
    fn parses_exact_amounts_without_float_rounding() {
        assert_eq!(
            parse_amount_to_atoms("1", InputUnit::Atho).unwrap(),
            ATOMS_PER_ATHO
        );
        assert_eq!(
            parse_amount_to_atoms("0.001", InputUnit::Atho).unwrap(),
            ATOMS_PER_MILLIATHO
        );
        assert_eq!(
            parse_amount_to_atoms("0.000001", InputUnit::Atho).unwrap(),
            ATOMS_PER_MICROATHO
        );
        assert_eq!(
            parse_amount_to_atoms("0.000000001", InputUnit::Atho).unwrap(),
            ATOMS_PER_NANOATHO
        );
        assert_eq!(
            parse_amount_to_atoms("0.000000000001", InputUnit::Atho).unwrap(),
            1
        );
        assert!(parse_amount_to_atoms("0.0000000000001", InputUnit::Atho).is_err());
        assert_eq!(
            parse_amount_to_atoms("0.000000001", InputUnit::MilliAtho).unwrap(),
            1
        );
        assert_eq!(
            parse_amount_to_atoms("0.000001", InputUnit::MicroAtho).unwrap(),
            1
        );
        assert_eq!(
            parse_amount_to_atoms("0.001", InputUnit::NanoAtho).unwrap(),
            1
        );
        assert_eq!(parse_amount_to_atoms("650", InputUnit::Atom).unwrap(), 650);
        assert!(parse_amount_to_atoms("0.1", InputUnit::Atom).is_err());
    }
}
