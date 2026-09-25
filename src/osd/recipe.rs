//! OSD recipes: complete `[osd]` presets applied with `voxtype setup osd --recipe`.
//!
//! A recipe is a TOML file whose `[osd]` table (layout, frame, and
//! `[[osd.visual.layers]]`) is merged into the user's config through
//! [`ConfigEditor::overlay_table`], the same comment-preserving, validated
//! write path `voxtype config set` uses. Hand-copying the keys instead is how
//! users ended up with duplicate-key configs the daemon refuses to load.

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::osd::style::user_then_system_roots;
use crate::tui::{ConfigEditor, EditorError, OverlayChange};

/// Where packages install the shipped recipes (`examples/osd-recipes`).
const SYSTEM_RECIPE_DIR: &str = "/usr/share/voxtype/osd-recipes";

/// A recipe found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// File stem; what `--recipe` accepts.
    pub name: String,
    pub path: PathBuf,
    /// The file's first comment line, when it has one.
    pub description: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecipeError {
    #[error("unknown OSD recipe '{name}'. Available recipes: {}", available_list(.available))]
    Unknown {
        name: String,
        available: Vec<String>,
    },

    #[error("recipe file {0} does not exist")]
    Missing(PathBuf),

    #[error("read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml_edit::TomlError,
    },

    #[error("{0} has no [osd] table, so there is nothing to apply")]
    NoOsdTable(PathBuf),

    #[error("config editor: {0}")]
    Editor(#[from] EditorError),
}

fn available_list(names: &[String]) -> String {
    if names.is_empty() {
        format!(
            "none installed (searched {})",
            recipe_roots()
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        names.join(", ")
    }
}

/// Recipe search roots, highest priority first: user config, user data, then
/// the system directory — the same order as OSD style packages.
pub fn recipe_roots() -> Vec<PathBuf> {
    user_then_system_roots("osd-recipes", Path::new(SYSTEM_RECIPE_DIR))
}

/// Every installed recipe in name order. A name present in several roots is
/// listed once, with the highest-priority copy.
pub fn list_recipes() -> Vec<Recipe> {
    list_recipes_in(&recipe_roots())
}

fn list_recipes_in(roots: &[PathBuf]) -> Vec<Recipe> {
    let mut found: std::collections::BTreeMap<String, Recipe> = Default::default();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let Some(name) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            found.entry(name.clone()).or_insert_with(|| Recipe {
                name,
                description: first_comment(&path),
                path,
            });
        }
    }
    found.into_values().collect()
}

fn first_comment(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .and_then(|l| l.strip_prefix('#'))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

/// Resolve `--recipe` to a file: an installed recipe name, or a path when the
/// argument contains a `/` or ends in `.toml`.
pub fn resolve_recipe(name_or_path: &str) -> Result<PathBuf, RecipeError> {
    resolve_recipe_in(name_or_path, &recipe_roots())
}

fn resolve_recipe_in(name_or_path: &str, roots: &[PathBuf]) -> Result<PathBuf, RecipeError> {
    if name_or_path.contains('/') || name_or_path.ends_with(".toml") {
        let path = PathBuf::from(name_or_path);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(RecipeError::Missing(path))
        };
    }
    let recipes = list_recipes_in(roots);
    recipes
        .iter()
        .find(|r| r.name == name_or_path)
        .map(|r| r.path.clone())
        .ok_or_else(|| RecipeError::Unknown {
            name: name_or_path.to_string(),
            available: recipes.into_iter().map(|r| r.name).collect(),
        })
}

/// What applying a recipe did (or, with `dry_run`, would do).
#[derive(Debug)]
pub struct ApplyOutcome {
    pub config: PathBuf,
    pub changes: Vec<OverlayChange>,
    /// Top-level keys other than `osd`; recipes only ever touch `[osd]`.
    pub ignored: Vec<String>,
    pub written: bool,
}

/// Merge the recipe's `[osd]` table into the config at `config_path`.
///
/// Keys the recipe sets replace the existing ones (arrays such as
/// `[[osd.visual.layers]]` are replaced wholesale); `[osd]` keys it doesn't
/// set are kept. The write is validated like any config edit and skipped
/// entirely when `dry_run` is set or nothing would change.
pub fn apply_recipe(
    recipe: &Path,
    config_path: PathBuf,
    dry_run: bool,
) -> Result<ApplyOutcome, RecipeError> {
    let text = fs::read_to_string(recipe).map_err(|source| RecipeError::Read {
        path: recipe.to_path_buf(),
        source,
    })?;
    let doc: DocumentMut = text.parse().map_err(|source| RecipeError::Parse {
        path: recipe.to_path_buf(),
        source,
    })?;
    let osd = doc
        .get("osd")
        .and_then(|i| i.as_table())
        .ok_or_else(|| RecipeError::NoOsdTable(recipe.to_path_buf()))?;
    let ignored = doc
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| k != "osd")
        .collect();

    let mut editor = ConfigEditor::load_from_path(config_path)?;
    let changes = editor.overlay_table("osd", osd);
    let written = !dry_run && !changes.is_empty();
    if written {
        editor.save()?;
    }
    Ok(ApplyOutcome {
        config: editor.path().to_path_buf(),
        changes,
        ignored,
        written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPE: &str = r#"# Test bars.
[osd]
frontend = "quickshell"
layout = "wide"

[osd.frame]
glow = true

[[osd.visual.layers]]
type = "bars"
source = "peak"

[[osd.visual.layers]]
type = "meter"
source = "peak"
"#;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn parse(text: &str) -> toml::Table {
        toml::from_str(text).unwrap_or_else(|e| panic!("config no longer parses: {e}\n{text}"))
    }

    #[test]
    fn apply_replaces_existing_osd_keys_without_duplicating_them() {
        let dir = tempfile::tempdir().unwrap();
        let recipe = write(dir.path(), "bars.toml", RECIPE);
        let config = write(
            dir.path(),
            "config.toml",
            r#"# my config
[osd]
# keep me
enabled = true
layout = "compact"

[osd.frame]
glow = false
border = "none"

[[osd.visual.layers]]
type = "waveform"
source = "rms"
"#,
        );

        let out = apply_recipe(&recipe, config.clone(), false).unwrap();
        assert!(out.written);
        let text = fs::read_to_string(&config).unwrap();
        let doc = parse(&text);
        let osd = doc["osd"].as_table().unwrap();

        assert_eq!(osd["layout"].as_str(), Some("wide"));
        assert_eq!(osd["frontend"].as_str(), Some("quickshell"));
        assert_eq!(osd["enabled"].as_bool(), Some(true), "unrelated key kept");
        let frame = osd["frame"].as_table().unwrap();
        assert_eq!(frame["glow"].as_bool(), Some(true));
        assert_eq!(frame["border"].as_str(), Some("none"), "unrelated key kept");
        let layers = osd["visual"]["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 2, "layers replaced, not appended: {text}");
        assert_eq!(layers[0]["type"].as_str(), Some("bars"));

        assert!(text.contains("# my config"), "comments preserved: {text}");
        assert!(text.contains("# keep me"), "comments preserved: {text}");
        assert_eq!(text.matches("[osd.frame]").count(), 1, "{text}");
        assert!(!text.contains("[osd.visual]\n"), "no empty header: {text}");

        let layout = out.changes.iter().find(|c| c.key == "osd.layout").unwrap();
        assert_eq!(layout.before.as_deref(), Some("\"compact\""));
        assert_eq!(layout.after, "\"wide\"");
        let frontend = out
            .changes
            .iter()
            .find(|c| c.key == "osd.frontend")
            .unwrap();
        assert_eq!(frontend.before, None);

        // Applying again changes nothing and still leaves one copy of each key.
        let again = apply_recipe(&recipe, config.clone(), false).unwrap();
        assert!(again.changes.is_empty(), "{:?}", again.changes);
        assert!(!again.written);
        let text = fs::read_to_string(&config).unwrap();
        assert_eq!(
            parse(&text)["osd"]["visual"]["layers"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn apply_into_config_without_osd_table() {
        let dir = tempfile::tempdir().unwrap();
        let recipe = write(dir.path(), "bars.toml", RECIPE);
        let config = write(dir.path(), "config.toml", "[hotkey]\nkey = \"F13\"\n");
        apply_recipe(&recipe, config.clone(), false).unwrap();
        let doc = parse(&fs::read_to_string(&config).unwrap());
        assert_eq!(doc["hotkey"]["key"].as_str(), Some("F13"));
        assert_eq!(doc["osd"]["layout"].as_str(), Some("wide"));
    }

    #[test]
    fn dry_run_reports_changes_but_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let recipe = write(dir.path(), "bars.toml", RECIPE);
        let original = "[osd]\nlayout = \"compact\"\n";
        let config = write(dir.path(), "config.toml", original);

        let out = apply_recipe(&recipe, config.clone(), true).unwrap();
        assert!(!out.written);
        assert!(out.changes.iter().any(|c| c.key == "osd.layout"));
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
        assert!(!dir.path().join("config.toml.bak").exists());
    }

    #[test]
    fn unknown_recipe_lists_the_available_names() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "showcase-bars.toml", RECIPE);
        write(dir.path(), "showcase-orbit.toml", RECIPE);
        let err = resolve_recipe_in("nope", &[dir.path().to_path_buf()]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown OSD recipe 'nope'"), "{msg}");
        assert!(msg.contains("showcase-bars, showcase-orbit"), "{msg}");
    }

    #[test]
    fn user_recipes_shadow_system_ones() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        let mine = write(user.path(), "showcase-bars.toml", "# Mine\n[osd]\n");
        write(system.path(), "showcase-bars.toml", "# Shipped\n[osd]\n");
        write(system.path(), "README.md", "not a recipe");
        let roots = [user.path().to_path_buf(), system.path().to_path_buf()];

        let recipes = list_recipes_in(&roots);
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].path, mine);
        assert_eq!(recipes[0].description.as_deref(), Some("Mine"));
        assert_eq!(resolve_recipe_in("showcase-bars", &roots).unwrap(), mine);
    }

    #[test]
    fn recipe_without_osd_table_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let recipe = write(dir.path(), "bad.toml", "[hotkey]\nkey = \"F1\"\n");
        let config = write(dir.path(), "config.toml", "");
        assert!(matches!(
            apply_recipe(&recipe, config, false),
            Err(RecipeError::NoOsdTable(_))
        ));
    }

    /// Every shipped recipe applies cleanly over the default config and
    /// survives the daemon's config validation.
    #[test]
    fn shipped_recipes_apply_over_the_default_config() {
        let shipped = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/osd-recipes");
        let recipes = list_recipes_in(&[shipped]);
        assert!(recipes.len() >= 4, "{recipes:?}");
        for recipe in recipes {
            let dir = tempfile::tempdir().unwrap();
            let config = write(
                dir.path(),
                "config.toml",
                &crate::config::default_config_content(),
            );
            let out = apply_recipe(&recipe.path, config, false)
                .unwrap_or_else(|e| panic!("{}: {e}", recipe.name));
            assert!(out.written, "{}", recipe.name);
            assert!(out.ignored.is_empty(), "{}", recipe.name);
        }
    }
}
