//! Structured list editors for outputs, terminal arguments and per-output wallpapers.

use std::collections::BTreeMap;

use crate::document::Toml;
use crate::draft::Field;

/// Column labels for one row, in the same order as its editable values.
pub fn columns(field: Field) -> &'static [&'static str] {
    match field {
        Field::Outputs => &[
            "settings-output-name",
            "settings-output-width",
            "settings-output-height",
            "settings-output-refresh",
            "settings-output-scale",
            "settings-output-x",
            "settings-output-y",
        ],
        Field::InputMethodCommand | Field::LauncherTerminal => &["settings-terminal-argument"],
        Field::WallpaperPerOutput => &["settings-output-name", "settings-wallpaper-path"],
        _ => &[],
    }
}

/// A new row populated from the authoritative schema defaults.
pub fn default_row(field: Field) -> Vec<String> {
    match field {
        Field::Outputs => {
            let output = inox_config::OutputConfig::default();
            vec![
                String::new(),
                output.width.to_string(),
                output.height.to_string(),
                output.refresh_hz.to_string(),
                output.scale.to_string(),
                String::new(),
                String::new(),
            ]
        }
        _ => vec![String::new(); columns(field).len()],
    }
}

/// Converts rows to TOML values without interpreting input as source or shell commands.
/// Returns `None` for incomplete rows, non-finite scales or duplicate output overrides.
pub fn toml(field: Field, rows: &[Vec<String>]) -> Option<Toml> {
    match field {
        Field::Outputs => rows
            .iter()
            .map(|row| output(row))
            .collect::<Option<Vec<_>>>()
            .map(Toml::Array),
        Field::InputMethodCommand | Field::LauncherTerminal => rows
            .iter()
            .map(|row| match row.as_slice() {
                [arg] => Some(Toml::String(arg.clone())),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(Toml::Array),
        Field::WallpaperPerOutput => {
            let mut entries = BTreeMap::new();
            for row in rows {
                let [name, path] = row.as_slice() else {
                    return None;
                };
                if name.is_empty()
                    || path.is_empty()
                    || entries
                        .insert(name.clone(), Toml::String(path.clone()))
                        .is_some()
                {
                    return None;
                }
            }
            Some(Toml::Table(entries))
        }
        _ => None,
    }
}

fn output(row: &[String]) -> Option<Toml> {
    let [name, width, height, refresh, scale, x, y] = row else {
        return None;
    };
    let scale = scale.parse::<f64>().ok()?;
    if !scale.is_finite() {
        return None;
    }
    let mut entries = BTreeMap::from([
        ("width".to_owned(), Toml::Integer(width.parse().ok()?)),
        ("height".to_owned(), Toml::Integer(height.parse().ok()?)),
        (
            "refresh_hz".to_owned(),
            Toml::Integer(refresh.parse().ok()?),
        ),
        ("scale".to_owned(), Toml::Decimal(format!("{scale:?}"))),
    ]);
    if !name.is_empty() {
        entries.insert("name".to_owned(), Toml::String(name.clone()));
    }
    if !x.is_empty() || !y.is_empty() {
        entries.insert(
            "position".to_owned(),
            Toml::Table(BTreeMap::from([
                ("x".to_owned(), Toml::Integer(x.parse().ok()?)),
                ("y".to_owned(), Toml::Integer(y.parse().ok()?)),
            ])),
        );
    }
    Some(Toml::Table(entries))
}
