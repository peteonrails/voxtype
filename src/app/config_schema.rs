//! `voxtype config schema [--json]`.
//!
//! The JSON form is the contract a settings UI codes against: every settable
//! key with its type, legal values, current value, and whether this binary
//! can honor it. See `src/config/schema.rs` for the allowlist itself.

use std::path::PathBuf;

use voxtype::config::schema::{self, KeyType};
use voxtype::config::Config;
use voxtype::tui::ConfigEditor;

use super::config_set::resolve_config_path_for_write;

pub(crate) fn run_config_schema(
    cli_override: Option<PathBuf>,
    config: &Config,
    json: bool,
) -> anyhow::Result<()> {
    let path = resolve_config_path_for_write(cli_override)?;
    let editor = ConfigEditor::load_from_path(path.clone())?;

    if json {
        let doc = schema::schema_json(config, &path, &editor);
        println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(());
    }

    println!("Voxtype config schema v{}", schema::SCHEMA_VERSION);
    println!("  voxtype:     {}", voxtype::cli::VERSION);
    println!("  config file: {}", path.display());
    println!("  engine:      {}", config.engine.name());
    println!();

    for section in schema::SECTIONS {
        let keys: Vec<_> = schema::scalar_keys()
            .filter(|s| s.section == *section)
            .collect();
        if keys.is_empty() {
            continue;
        }
        println!("{}", section);
        for spec in keys {
            let value = schema::resolve(spec.key, config).unwrap_or(serde_json::Value::Null);
            let value = match &value {
                serde_json::Value::Null => "unset".to_string(),
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let mut notes: Vec<String> = Vec::new();
            if let Some(engine) = spec.engine {
                notes.push(format!("engine={}", engine));
            }
            if !spec.compiled() {
                notes.push("not compiled in".to_string());
            }
            if spec.restart_required {
                notes.push("needs restart".to_string());
            }
            println!("  {}", spec.key);
            println!("    type    {}", describe_type(spec.ty));
            println!("    value   {}", value);
            if !notes.is_empty() {
                println!("    notes   {}", notes.join(", "));
            }
            println!("    {}", spec.description);
        }
        println!();
    }

    if !config.text.replacements.is_empty() {
        println!("Replacements ({})", schema::REPLACEMENTS_TABLE);
        let mut entries: Vec<_> = config.text.replacements.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (from, to) in entries {
            println!("  {} -> {}", from, to);
        }
        println!();
    }

    println!("Set a value with: voxtype config set <KEY> <VALUE>");
    Ok(())
}

fn describe_type(ty: KeyType) -> String {
    match ty {
        KeyType::Bool => "bool (true|false)".to_string(),
        KeyType::Int { min, max } => format!("int {}..{}", min, max),
        KeyType::Float { min, max } => format!("float {}..{}", min, max),
        KeyType::String => "string".to_string(),
        KeyType::Enum { choices, open } => {
            let list = choices.join(" | ");
            if open {
                format!("string, commonly {}", list)
            } else {
                list
            }
        }
        KeyType::DynamicEnum { source } => {
            format!("string (see `voxtype info {} --json`)", source)
        }
        KeyType::MapString => "map of string to string".to_string(),
    }
}
