//! One config file that can pull in others.
//!
//! A project should not have to repeat itself: a base file holds the common shape, and each
//! project includes it and overrides what differs. The rule is deliberately the boring one,
//! because a clever merge is a merge nobody can predict:
//!
//! - includes are applied first, and the including file wins for any single value;
//! - lists are concatenated, include entries first, with duplicates removed.
//!
//! A cycle is refused rather than followed, because two files including each other has no
//! meaning and would otherwise hang the parser.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::ConfigError;

/// Load a config file, resolving every include it names.
///
/// Returns merged TOML text, ready for the recipe or stack parser. Working at the text level
/// keeps those parsers exactly as they are, and means the merge cannot introduce a field that
/// nothing validates.
pub fn load_merged(path: &Path) -> Result<String, ConfigError> {
    let mut visiting = BTreeSet::new();
    let merged = merge_file(path, &mut visiting)?;

    toml::to_string(&merged).map_err(|error| ConfigError::Render(error.to_string()))
}

fn merge_file(path: &Path, visiting: &mut BTreeSet<PathBuf>) -> Result<toml::Value, ConfigError> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visiting.insert(canonical.clone()) {
        return Err(ConfigError::IncludeCycle {
            path: path.display().to_string(),
        });
    }

    let text = std::fs::read_to_string(path).map_err(|error| ConfigError::CannotRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    let mut value: toml::Value =
        toml::from_str(&text).map_err(|error| ConfigError::Parse(error.to_string()))?;

    // The include list is removed before merging, so it never reaches the result.
    let includes = match value
        .as_table_mut()
        .and_then(|table| table.remove("include"))
    {
        Some(toml::Value::Array(entries)) => entries
            .into_iter()
            .map(|entry| match entry {
                toml::Value::String(text) => Ok(text),
                _ => Err(ConfigError::InvalidValue {
                    field: "include",
                    reason: "every entry must be a string".to_owned(),
                }),
            })
            .collect::<Result<Vec<String>, ConfigError>>()?,
        Some(_) => {
            return Err(ConfigError::InvalidValue {
                field: "include",
                reason: "must be a list of paths".to_owned(),
            });
        }
        None => Vec::new(),
    };

    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut merged = toml::Value::Table(toml::map::Map::new());

    for include in includes {
        let child = merge_file(&directory.join(&include), visiting)?;
        merged = merge_into(merged, child);
    }

    visiting.remove(&canonical);
    Ok(merge_into(merged, value))
}

/// Merge two documents: arrays concatenate, anything else in the later document wins.
fn merge_into(earlier: toml::Value, later: toml::Value) -> toml::Value {
    match (earlier, later) {
        (toml::Value::Table(mut left), toml::Value::Table(right)) => {
            for (key, value) in right {
                match left.remove(&key) {
                    Some(existing) => {
                        left.insert(key, merge_into(existing, value));
                    }
                    None => {
                        left.insert(key, value);
                    }
                }
            }
            toml::Value::Table(left)
        }
        (toml::Value::Array(left), toml::Value::Array(right)) => {
            let mut combined = left;
            for item in right {
                if !combined.contains(&item) {
                    combined.push(item);
                }
            }
            toml::Value::Array(combined)
        }
        (_, later) => later,
    }
}
