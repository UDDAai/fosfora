//! Display names the user gives effects from the web controller.
//!
//! Presets, scenes and the engine all keep the real effect name; an alias only
//! changes what the controller shows. The map lives in one small per-user file,
//! `effect-names.json`, beside the other settings.

use std::collections::BTreeMap;
use std::path::PathBuf;

pub const FILE_NAME: &str = "effect-names.json";
/// Longest alias, in characters.
pub const MAX_ALIAS_CHARS: usize = 64;

#[derive(Debug, Default)]
pub struct EffectAliases {
    path: PathBuf,
    /// Real effect name -> alias.
    names: BTreeMap<String, String>,
}

/// Trim and check one alias. `Ok("")` means "no alias".
fn clean_alias(raw: &str) -> Result<String, String> {
    let alias = raw.trim();
    if alias.chars().count() > MAX_ALIAS_CHARS {
        return Err(format!(
            "An effect name can be at most {MAX_ALIAS_CHARS} characters"
        ));
    }
    if alias.chars().any(char::is_control) {
        return Err("An effect name cannot contain control characters".into());
    }
    Ok(alias.to_string())
}

impl EffectAliases {
    /// The user's file under the config root.
    pub fn load_default() -> Self {
        Self::load(crate::paths::config_root().join(FILE_NAME))
    }

    /// Read `path`. A missing file is an empty map. A file that does not parse
    /// is set aside as `<name>.bad` so the next save cannot overwrite it, and
    /// the map starts empty.
    pub fn load(path: PathBuf) -> Self {
        let names = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<BTreeMap<String, String>>(&text) {
                Ok(parsed) => parsed
                    .into_iter()
                    .filter_map(|(real, alias)| match clean_alias(&alias) {
                        Ok(alias) if !alias.is_empty() => Some((real, alias)),
                        _ => {
                            log::warn!("Effect names: ignoring the name for {real}");
                            None
                        }
                    })
                    .collect(),
                Err(err) => {
                    let mut bad = path.as_os_str().to_owned();
                    bad.push(".bad");
                    log::warn!(
                        "Effect names: {} is not valid ({err}); moved to {} and starting empty",
                        path.display(),
                        PathBuf::from(&bad).display()
                    );
                    let _ = std::fs::rename(&path, PathBuf::from(bad));
                    BTreeMap::new()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(err) => {
                log::warn!("Effect names: cannot read {}: {err}", path.display());
                BTreeMap::new()
            }
        };
        Self { path, names }
    }

    /// What the controller shows for `real`: its alias, or `real` itself.
    pub fn display_name<'a>(&'a self, real: &'a str) -> &'a str {
        self.names.get(real).map_or(real, String::as_str)
    }

    #[cfg(test)]
    pub fn has_alias(&self, real: &str) -> bool {
        self.names.contains_key(real)
    }

    /// Give `real` the alias `raw`, or remove its alias when `raw` is empty or
    /// equals `real`. `all_real_names` is every effect the engine knows; an alias
    /// may not match another effect's real name or alias (ignoring case).
    /// Nothing changes in memory or on disk unless the whole call succeeds.
    pub fn set<'a>(
        &mut self,
        real: &str,
        raw: &str,
        all_real_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), String> {
        let alias = clean_alias(raw)?;
        let mut next = self.names.clone();
        if alias.is_empty() || alias == real {
            next.remove(real);
        } else {
            let mut known = false;
            let wanted = alias.to_lowercase();
            for other in all_real_names {
                if other == real {
                    known = true;
                } else if other.to_lowercase() == wanted {
                    return Err(format!("{alias} is already the name of an effect"));
                }
            }
            if !known {
                return Err(format!("{real} is not an effect"));
            }
            if let Some((other, _)) = self
                .names
                .iter()
                .find(|(other, used)| other.as_str() != real && used.to_lowercase() == wanted)
            {
                return Err(format!("{alias} is already the name of {other}"));
            }
            next.insert(real.to_string(), alias);
        }
        if next == self.names {
            return Ok(());
        }
        let text = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Could not save effect names: {e}"))?;
        }
        crate::paths::write_atomic(&self.path, &text)
            .map_err(|e| format!("Could not save effect names: {e}"))?;
        self.names = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh temp dir per case; the house pattern (no tempfile dep in release deps).
    fn temp_dir(case: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fosfora_effect_names_{case}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    const EFFECTS: [&str; 3] = ["Astrolabe", "Aurora", "Plasma"];

    #[test]
    fn a_name_round_trips_through_the_file() {
        let dir = temp_dir("round_trip");
        let path = dir.join(FILE_NAME);
        let mut aliases = EffectAliases::load(path.clone());
        assert_eq!(aliases.display_name("Astrolabe"), "Astrolabe");
        aliases.set("Astrolabe", "  Ice Clock ", EFFECTS).unwrap();
        assert_eq!(aliases.display_name("Astrolabe"), "Ice Clock");

        let reloaded = EffectAliases::load(path);
        assert_eq!(reloaded.display_name("Astrolabe"), "Ice Clock");
        assert_eq!(reloaded.display_name("Aurora"), "Aurora");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_alias_removes_the_entry() {
        let dir = temp_dir("empty");
        let path = dir.join(FILE_NAME);
        let mut aliases = EffectAliases::load(path.clone());
        aliases.set("Aurora", "Northern", EFFECTS).unwrap();
        aliases.set("Aurora", "   ", EFFECTS).unwrap();
        assert!(!aliases.has_alias("Aurora"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            "{}",
            "the entry is gone from the file too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_alias_equal_to_the_real_name_is_a_reset() {
        let dir = temp_dir("same");
        let mut aliases = EffectAliases::load(dir.join(FILE_NAME));
        aliases.set("Aurora", "Northern", EFFECTS).unwrap();
        aliases.set("Aurora", "Aurora", EFFECTS).unwrap();
        assert!(!aliases.has_alias("Aurora"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_collision_with_another_effect_is_refused() {
        let dir = temp_dir("collision");
        let path = dir.join(FILE_NAME);
        let mut aliases = EffectAliases::load(path.clone());
        // Another effect's real name, any case.
        assert!(aliases.set("Aurora", "plasma", EFFECTS).is_err());
        // Another effect's alias, any case.
        aliases.set("Plasma", "Lava", EFFECTS).unwrap();
        assert!(aliases.set("Aurora", "LAVA", EFFECTS).is_err());
        // Its own alias again is fine.
        aliases.set("Plasma", "lava", EFFECTS).unwrap();
        assert_eq!(aliases.display_name("Aurora"), "Aurora");
        assert_eq!(aliases.display_name("Plasma"), "lava");
        // The refusal left the file as it was.
        let reloaded = EffectAliases::load(path);
        assert_eq!(reloaded.display_name("Plasma"), "lava");
        assert!(!reloaded.has_alias("Aurora"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_input_is_refused() {
        let dir = temp_dir("bad_input");
        let mut aliases = EffectAliases::load(dir.join(FILE_NAME));
        assert!(aliases.set("Aurora", &"x".repeat(65), EFFECTS).is_err());
        assert!(aliases.set("Aurora", &"x".repeat(64), EFFECTS).is_ok());
        assert!(aliases.set("Aurora", "two\nlines", EFFECTS).is_err());
        assert!(aliases.set("Aurora", "tab\there", EFFECTS).is_err());
        assert!(aliases.set("Nope", "Whatever", EFFECTS).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_file_is_set_aside_and_the_map_starts_empty() {
        let dir = temp_dir("bad_file");
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "{ not json").unwrap();
        let mut aliases = EffectAliases::load(path.clone());
        assert_eq!(aliases.display_name("Aurora"), "Aurora");
        assert_eq!(
            std::fs::read_to_string(dir.join("effect-names.json.bad")).unwrap(),
            "{ not json"
        );
        // Saving afterwards works and does not touch the set-aside copy.
        aliases.set("Aurora", "Northern", EFFECTS).unwrap();
        assert_eq!(EffectAliases::load(path).display_name("Aurora"), "Northern");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_entries_in_a_good_file_are_skipped() {
        let dir = temp_dir("skip");
        let path = dir.join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"Aurora":"Northern","Plasma":"bad\u0007","Astrolabe":""}"#,
        )
        .unwrap();
        let aliases = EffectAliases::load(path);
        assert_eq!(aliases.display_name("Aurora"), "Northern");
        assert!(!aliases.has_alias("Plasma"));
        assert!(!aliases.has_alias("Astrolabe"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
