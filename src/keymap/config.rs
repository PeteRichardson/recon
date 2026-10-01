//! The `[keymap]` table of `config.toml`: how it is read, and how it becomes
//! the keymap in force.
//!
//! Here and not in `config.rs` (#409). Nothing else in the file format needs
//! a hand-written `Deserialize`, and `config.rs` held the CLI, the schema, the
//! precedence chain and this — about 180 lines that only the keymap reads.

use crate::config::{ConfigError, config_path};
use serde::Deserialize;
use std::fmt;

/// The `[keymap]` table: which keys reach which action.
///
/// Action to key, and not the other way round, for two reasons. It is the
/// direction `--print-keymap` prints, so a pasted line reads as it was
/// printed. And an action may hold several keys, which a key cannot.
///
/// A `BTreeMap` rather than named fields: the keys are action names, there are
/// about ninety of them, and `deny_unknown_fields` cannot help here — an
/// unknown action is caught by `Keymap::new`, which can say which names exist.
///
/// `Deserialize` is hand-written, not derived, and not via `#[serde(flatten)]`
/// either (#61). `flatten` buffers the whole table into a generic value
/// before `Keys` ever sees it, so a malformed entry (`'global.quit' = 42`)
/// fails with the position pinned to the `[keymap]` header rather than the
/// offending line, and a message naming the private `Keys` type instead of
/// the action. An unflattened field with the same bad value reports the
/// right line and a legible message.
///
/// So this decodes the table directly, one entry at a time, with
/// [`BindingSeed`] threading the action's name into the value's own error —
/// see its doc comment for how.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeymapConfig {
    pub bindings: std::collections::BTreeMap<String, Vec<String>>,
}

impl<'de> Deserialize<'de> for KeymapConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct KeymapVisitor;

        impl<'de> serde::de::Visitor<'de> for KeymapVisitor {
            type Value = KeymapConfig;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a table mapping each action to one key or an array of keys")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut bindings = std::collections::BTreeMap::new();
                while let Some(action) = map.next_key::<String>()? {
                    let keys = map.next_value_seed(BindingSeed { action: &action })?;
                    bindings.insert(action, keys);
                }
                Ok(KeymapConfig { bindings })
            }
        }

        deserializer.deserialize_map(KeymapVisitor)
    }
}

/// Decodes one `[keymap]` value against the action it belongs to.
///
/// A [`serde::de::DeserializeSeed`] rather than a plain `Deserialize` type,
/// because the action's name has to reach the error — `Deserialize` alone
/// carries no state, and `map_err`-ing after the fact (the alternative the
/// review offered) would replace the position-carrying error the deserializer
/// already built with a fresh, unpositioned one. Threading the name in here
/// instead means the error `KeyOrKeys::expecting` writes is the one the
/// deserializer reports natively, position and all.
struct BindingSeed<'a> {
    action: &'a str,
}

impl<'de> serde::de::DeserializeSeed<'de> for BindingSeed<'_> {
    type Value = Vec<String>;

    fn deserialize<D>(self, deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(KeyOrKeys {
            action: self.action,
        })
    }
}

/// One `[keymap]` value: a bare string for a single key, or an array for
/// several. `expecting` names the action, so a value that is neither —
/// `42`, a table — is refused with a message naming what was wrong and
/// where, not a Rust type.
struct KeyOrKeys<'a> {
    action: &'a str,
}

impl<'de> serde::de::Visitor<'de> for KeyOrKeys<'_> {
    type Value = Vec<String>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "one key or an array of keys for {:?} in [keymap]",
            self.action
        )
    }

    fn visit_str<E>(self, v: &str) -> Result<Vec<String>, E>
    where
        E: serde::de::Error,
    {
        Ok(vec![v.to_string()])
    }

    fn visit_string<E>(self, v: String) -> Result<Vec<String>, E>
    where
        E: serde::de::Error,
    {
        Ok(vec![v])
    }

    /// A table where a key belongs is almost always an action name written
    /// without quotes (#366): TOML reads `global.quit = 'q'` as a table
    /// `global` holding `quit`. Without this the message named "global" as
    /// the action and "map" as the fault, which points nowhere useful.
    ///
    /// Follows the table down while it holds one entry, so the name in the
    /// message is the one the user wrote: `global.hide.toggle`, not
    /// `global.hide`.
    fn visit_map<A>(self, mut map: A) -> Result<Vec<String>, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        use serde::de::Error;

        let mut name = self.action.to_string();
        if let Some(key) = map.next_key::<String>()? {
            name = format!("{name}.{key}");
            let mut value: toml::Value = map.next_value()?;
            while let toml::Value::Table(table) = value {
                let mut entries = table.into_iter();
                match (entries.next(), entries.next()) {
                    (Some((key, inner)), None) => {
                        name = format!("{name}.{key}");
                        value = inner;
                    }
                    _ => break,
                }
            }
        }
        Err(A::Error::custom(format!(
            "{:?} in [keymap] is a table, not a key; quote an action name that holds a dot: '{name}' = …",
            self.action
        )))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Vec<String>, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut keys = Vec::new();
        while let Some(key) = seq.next_element_seed(KeyLabelSeed {
            action: self.action,
        })? {
            keys.push(key);
        }
        Ok(keys)
    }
}

/// Decodes one array element of a `[keymap]` value, so an array holding a
/// non-string (`['u', 5]`) is refused naming the action too, the same as a
/// bare malformed value is.
struct KeyLabelSeed<'a> {
    action: &'a str,
}

impl<'de> serde::de::DeserializeSeed<'de> for KeyLabelSeed<'_> {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<String, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(KeyLabel {
            action: self.action,
        })
    }
}

/// One key spelling inside a `[keymap]` array — see [`KeyLabelSeed`].
struct KeyLabel<'a> {
    action: &'a str,
}

impl serde::de::Visitor<'_> for KeyLabel<'_> {
    type Value = String;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a key spelling for {:?} in [keymap]", self.action)
    }

    fn visit_str<E>(self, v: &str) -> Result<String, E>
    where
        E: serde::de::Error,
    {
        Ok(v.to_string())
    }

    fn visit_string<E>(self, v: String) -> Result<String, E>
    where
        E: serde::de::Error,
    {
        Ok(v)
    }
}

/// Every binding in force, and whatever the file cost that is worth
/// saying out loud. `warnings` is `Config::warnings()`: whether the
/// reserved-key notice goes to stderr.
///
/// Three steps, in this order. `Keymap::new` folds `[keymap]` over the
/// defaults, leaving a contested key claimed by two actions. `check`
/// reads that and says which claims are faults, which are costs, and
/// which rows must go. Then the rows go, which is what makes a written
/// line win — see `Keymap::evict`.
///
/// `startup::start` calls this before the terminal comes up, so an error still
/// reaches a screen that a user can read, and the warnings are in hand
/// before `Muted` starts dropping records (#246).
///
/// # Errors
///
/// [`ConfigError::UnknownAction`], [`ConfigError::BadKeyLabel`] or
/// [`ConfigError::Inconsistent`].
pub fn build(
    overlay: &KeymapConfig,
    warnings: bool,
) -> Result<(crate::keymap::Keymap, Vec<String>), ConfigError> {
    let (mut keymap, reserved) =
        crate::keymap::Keymap::new(overlay).map_err(|err| err.in_file(config_path()))?;

    // Logged here rather than inside `Keymap::new`, which is not told
    // whether the user asked for silence.
    // Still on stderr and not in the panel: binding `-` or `:` is a
    // deliberate choice that no keymap edit answers, so a panel meaning
    // "correct this" would ask again at every start.
    if warnings {
        for warning in reserved {
            log::warn!("{warning}");
        }
    }

    let written: Vec<crate::keymap::ActionId> = overlay
        .bindings
        .keys()
        .filter_map(|name| crate::keymap::action_named(name))
        .collect();
    let report = crate::keymap::check::check(&keymap, &written);

    // Rendered here, at the boundary. `ConfigError` is public API and
    // `check::Problem` is `pub(crate)`, so a variant carrying the type
    // itself is E0446 — a private type in a public interface — and will
    // not compile. Rendering also keeps `Problem`'s `Display` the single
    // place any of this is worded.
    if !report.errors().is_empty() {
        return Err(ConfigError::Inconsistent {
            problems: report.errors().iter().map(ToString::to_string).collect(),
            path: config_path(),
        });
    }
    keymap.evict(report.evict());
    Ok((
        keymap,
        report.warnings().iter().map(ToString::to_string).collect(),
    ))
}
