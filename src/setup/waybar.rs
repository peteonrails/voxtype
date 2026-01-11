//! Waybar configuration generation for voxtype

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::error::VoxtypeError;

/// Get the user's config directory (~/.config on Linux)
fn get_user_config_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from(".config"))
        })
}

/// Get the default waybar config path
fn get_config_path() -> PathBuf {
    get_user_config_dir().join("waybar").join("config.jsonc")
}

/// Get the default waybar style path
fn get_style_path() -> PathBuf {
    get_user_config_dir().join("waybar").join("style.css")
}

/// Install waybar integration (inject config and CSS)
pub fn install() -> Result<(), VoxtypeError> {
    let config_path = get_config_path();
    let style_path = get_style_path();

    // Check if waybar config exists
    if !config_path.exists() {
        // Try without .jsonc extension
        let alt_path = config_path.with_extension("");
        if !alt_path.exists() {
            eprintln!("Waybar config not found at:");
            eprintln!("  {}", config_path.display());
            eprintln!("  {}", alt_path.display());
            eprintln!("\nPlease install Waybar first or create a config file.");
            return Err(VoxtypeError::Config("Waybar config not found".into()));
        }
    }

    let config_path = if config_path.exists() {
        config_path
    } else {
        config_path.with_extension("")
    };

    // Read current config
    let config_content = fs::read_to_string(&config_path)
        .map_err(|e| VoxtypeError::Config(format!("Failed to read waybar config: {}", e)))?;

    // Check if voxtype module already exists
    if config_content.contains("\"custom/voxtype\"") {
        println!("Voxtype module already exists in Waybar config.");
        println!("Use --uninstall first if you want to reinstall.");
        return Ok(());
    }

    // Prompt for confirmation
    println!("This will modify your Waybar configuration:");
    println!("  Config: {}", config_path.display());
    println!("  Style:  {}", style_path.display());
    println!("\nA backup will be created before making changes.");
    print!("\nProceed with installation? [y/N] ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Installation cancelled.");
        return Ok(());
    }

    // Create backup
    let backup_path = format!("{}.voxtype-backup", config_path.display());
    fs::copy(&config_path, &backup_path)
        .map_err(|e| VoxtypeError::Config(format!("Failed to create backup: {}", e)))?;
    println!("Created backup: {}", backup_path);

    // Inject module into config
    let new_config = inject_module_into_config(&config_content)?;
    fs::write(&config_path, new_config)
        .map_err(|e| VoxtypeError::Config(format!("Failed to write config: {}", e)))?;
    println!("Added voxtype module to Waybar config.");

    // Add CSS if style file exists and doesn't already have voxtype styles
    if style_path.exists() {
        let style_content = fs::read_to_string(&style_path).unwrap_or_default();
        if !style_content.contains("#custom-voxtype") {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(&style_path)
                .map_err(|e| VoxtypeError::Config(format!("Failed to open style.css: {}", e)))?;
            writeln!(file, "\n{}", get_css_config())
                .map_err(|e| VoxtypeError::Config(format!("Failed to write CSS: {}", e)))?;
            println!("Added voxtype styling to Waybar CSS.");
        }
    }

    println!("\nWaybar integration installed successfully!");
    println!("Restart Waybar to see the voxtype status widget:");
    println!("  pkill waybar && waybar &");

    Ok(())
}

/// Uninstall waybar integration (remove config and CSS)
pub fn uninstall() -> Result<(), VoxtypeError> {
    let config_path = get_config_path();
    let style_path = get_style_path();

    let config_path = if config_path.exists() {
        config_path
    } else {
        let alt = config_path.with_extension("");
        if alt.exists() {
            alt
        } else {
            println!("Waybar config not found, nothing to uninstall.");
            return Ok(());
        }
    };

    let config_content = fs::read_to_string(&config_path)
        .map_err(|e| VoxtypeError::Config(format!("Failed to read waybar config: {}", e)))?;

    if !config_content.contains("\"custom/voxtype\"") {
        println!("Voxtype module not found in Waybar config.");
        return Ok(());
    }

    // Prompt for confirmation
    print!("Remove voxtype from Waybar configuration? [y/N] ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Uninstall cancelled.");
        return Ok(());
    }

    // Remove module from config
    let new_config = remove_module_from_config(&config_content);
    fs::write(&config_path, new_config)
        .map_err(|e| VoxtypeError::Config(format!("Failed to write config: {}", e)))?;
    println!("Removed voxtype module from Waybar config.");

    // Remove CSS
    if style_path.exists() {
        let style_content = fs::read_to_string(&style_path).unwrap_or_default();
        if style_content.contains("#custom-voxtype") {
            let new_style = remove_css_from_style(&style_content);
            fs::write(&style_path, new_style)
                .map_err(|e| VoxtypeError::Config(format!("Failed to write style.css: {}", e)))?;
            println!("Removed voxtype styling from Waybar CSS.");
        }
    }

    println!("\nWaybar integration removed.");
    println!("Restart Waybar to apply changes:");
    println!("  pkill waybar && waybar &");

    Ok(())
}

/// Inject the voxtype module into waybar config content
fn inject_module_into_config(content: &str) -> Result<String, VoxtypeError> {
    let mut result = content.to_string();

    // Add "custom/voxtype" to modules-right array
    // Look for "modules-right": [...] and insert at the beginning
    if let Some(pos) = result.find("\"modules-right\"") {
        if let Some(bracket_pos) = result[pos..].find('[') {
            let insert_pos = pos + bracket_pos + 1;
            // Skip whitespace/newline after [
            let after_bracket = &result[insert_pos..];
            let skip = after_bracket
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
            let insert_pos = insert_pos + skip;

            // Insert the module reference
            result.insert_str(insert_pos, "\"custom/voxtype\",\n        ");
        }
    } else if let Some(pos) = result.find("\"modules-left\"") {
        // Fallback to modules-left if modules-right doesn't exist
        if let Some(bracket_pos) = result[pos..].find('[') {
            let insert_pos = pos + bracket_pos + 1;
            let after_bracket = &result[insert_pos..];
            let skip = after_bracket
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
            let insert_pos = insert_pos + skip;
            result.insert_str(insert_pos, "\"custom/voxtype\",\n        ");
        }
    }

    // Add the module definition before the final }
    // Find the last } in the file (the closing brace of the root object)
    if let Some(last_brace) = result.rfind('}') {
        // Check if we need to add a comma after the previous module
        // Look backwards from the last brace to find the previous }
        let before_last = &result[..last_brace];
        let needs_comma = before_last.trim_end().ends_with('}');

        let module_def = if needs_comma {
            r#",

    "custom/voxtype": {
        "exec": "voxtype status --follow --format json",
        "return-type": "json",
        "format": "{}",
        "tooltip": true,
        "on-click": "systemctl --user restart voxtype"
    }
"#
        } else {
            r#"
    "custom/voxtype": {
        "exec": "voxtype status --follow --format json",
        "return-type": "json",
        "format": "{}",
        "tooltip": true,
        "on-click": "systemctl --user restart voxtype"
    }
"#
        };

        // Find where to insert - right before the final }
        // But we want to insert after the last content, which is before any trailing whitespace and the final }
        let insert_pos = before_last.trim_end().len();
        result.insert_str(insert_pos, module_def);
    }

    Ok(result)
}

/// Remove the voxtype module from waybar config content
fn remove_module_from_config(content: &str) -> String {
    let mut result = content.to_string();

    // IMPORTANT: Remove the definition block FIRST, before removing array references
    // Otherwise the regex removes the key and we can't find the block anymore

    // Remove the module definition block using a more careful approach
    // Find the start of the block by looking for the key definition (with colon)
    // This distinguishes from array references like ["custom/voxtype", ...]
    let definition_pattern = regex::Regex::new(r#""custom/voxtype"\s*:"#).unwrap();
    if let Some(mat) = definition_pattern.find(&result) {
        let start = mat.start();
        // Find the opening { after "custom/voxtype":
        if let Some(brace_offset) = result[mat.end()..].find('{') {
            let brace_start = mat.end() + brace_offset;
            // Find the matching closing brace by counting
            let mut depth = 1;
            let mut end = brace_start + 1;
            let chars: Vec<char> = result[brace_start + 1..].chars().collect();
            for (i, ch) in chars.iter().enumerate() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = brace_start + 1 + i + 1; // +1 to include the }
                            break;
                        }
                    }
                    _ => {}
                }
            }

            // Find the actual start (including any leading whitespace/comma)
            // Look backwards for comma or whitespace to remove
            let before = &result[..start];
            let trimmed_before = before.trim_end();
            let actual_start = if trimmed_before.ends_with(',') {
                trimmed_before.len() - 1
            } else {
                // Include leading whitespace
                trimmed_before.len()
            };

            // Check if there's a trailing comma to remove
            let after = &result[end..];
            let trimmed_after = after.trim_start();
            let trailing_comma = if trimmed_after.starts_with(',') {
                after.find(',').map(|i| end + i + 1)
            } else {
                None
            };

            let actual_end = trailing_comma.unwrap_or(end);

            // Remove the block
            result = format!("{}{}", &result[..actual_start], &result[actual_end..]);
        }
    }

    // Now remove "custom/voxtype" from module arrays (modules-right, modules-left, etc.)
    // This handles the reference that was added to the array
    result = regex::Regex::new(r#""custom/voxtype",?\s*\n?\s*"#)
        .unwrap()
        .replace_all(&result, "")
        .to_string();

    result
}

/// Remove voxtype CSS from style content
fn remove_css_from_style(content: &str) -> String {
    let mut result = content.to_string();

    // Remove all #custom-voxtype CSS blocks using brace matching
    while let Some(start) = result.find("#custom-voxtype") {
        // Find the opening brace
        if let Some(brace_offset) = result[start..].find('{') {
            let brace_start = start + brace_offset;
            // Find matching closing brace
            let mut depth = 1;
            let mut end = brace_start + 1;
            for (i, ch) in result[brace_start + 1..].chars().enumerate() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = brace_start + 1 + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            // Include trailing whitespace
            let after = &result[end..];
            let trailing_ws = after.chars().take_while(|c| c.is_whitespace()).count();
            // Remove the block including leading whitespace
            let before = &result[..start];
            let leading_ws = before
                .chars()
                .rev()
                .take_while(|c| *c == ' ' || *c == '\t' || *c == '\n')
                .count();
            let actual_start = start.saturating_sub(leading_ws);
            let actual_end = end + trailing_ws;
            result = format!("{}{}", &result[..actual_start], &result[actual_end..]);
        } else {
            break;
        }
    }

    // Remove @keyframes pulse block (that we added)
    // Only remove if it's our simple pulse animation
    if let Some(start) = result.find("@keyframes pulse") {
        if let Some(brace_offset) = result[start..].find('{') {
            let brace_start = start + brace_offset;
            let mut depth = 1;
            let mut end = brace_start + 1;
            for (i, ch) in result[brace_start + 1..].chars().enumerate() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = brace_start + 1 + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            // Include trailing whitespace
            let after = &result[end..];
            let trailing_ws = after.chars().take_while(|c| c.is_whitespace()).count();
            let before = &result[..start];
            let leading_ws = before
                .chars()
                .rev()
                .take_while(|c| *c == ' ' || *c == '\t' || *c == '\n')
                .count();
            let actual_start = start.saturating_sub(leading_ws);
            let actual_end = end + trailing_ws;
            result = format!("{}{}", &result[..actual_start], &result[actual_end..]);
        }
    }

    // Remove any commented out voxtype blocks
    result = regex::Regex::new(r#"(?s)/\*[^*]*[Vv]oxtype[^*]*\*/"#)
        .unwrap()
        .replace_all(&result, "")
        .to_string();

    // Clean up multiple blank lines
    result = regex::Regex::new(r#"\n{3,}"#)
        .unwrap()
        .replace_all(&result, "\n\n")
        .to_string();

    result
}

/// Generate and print Waybar configuration
pub fn print_config() {
    println!("Waybar Configuration for Voxtype\n");
    println!("================================\n");

    println!("1. Add this to your Waybar config (usually ~/.config/waybar/config):\n");
    println!("   In the \"modules-right\" (or left/center) array, add: \"custom/voxtype\"\n");

    println!("   Then add this module configuration:\n");
    println!(
        r#"   "custom/voxtype": {{
       "exec": "voxtype status --follow --format json",
       "return-type": "json",
       "format": "{{}}",
       "tooltip": true,
       "on-click": "systemctl --user restart voxtype"
   }}"#
    );

    println!("\n\n2. Add this to your Waybar style.css:\n");
    println!(
        r#"   #custom-voxtype {{
       padding: 0 10px;
   }}

   #custom-voxtype.recording {{
       color: #ff5555;
       animation: pulse 1s ease-in-out infinite;
   }}

   #custom-voxtype.transcribing {{
       color: #f1fa8c;
   }}

   #custom-voxtype.idle {{
       color: #50fa7b;
   }}

   #custom-voxtype.stopped {{
       color: #6272a4;
   }}

   @keyframes pulse {{
       0%, 100% {{ opacity: 1; }}
       50% {{ opacity: 0.5; }}
   }}"#
    );

    println!("\n\n3. Enable state file in voxtype config (~/.config/voxtype/config.toml):\n");
    println!("   state_file = \"auto\"\n");

    println!("\n4. Restart Waybar to apply changes:\n");
    println!("   killall waybar && waybar &\n");

    println!("---");
    println!("\nCustomizing Icons:");
    println!("------------------");
    println!(
        "Voxtype outputs an \"alt\" field in JSON that enables Waybar's format-icons feature."
    );
    println!("To use custom icons (e.g., Nerd Fonts), configure your Waybar module like this:\n");
    println!(
        r#"   "custom/voxtype": {{
       "exec": "voxtype status --follow --format json",
       "return-type": "json",
       "format": "{{icon}}",
       "format-icons": {{
           "idle": "\uf130",
           "recording": "\uf111",
           "transcribing": "\uf110",
           "stopped": "\uf131"
       }},
       "tooltip": true
   }}"#
    );
    println!("\n   Nerd Font codepoints: U+F130 (mic), U+F111 (dot), U+F110 (spinner), U+F131 (mic-slash)");
    println!("\nAlternatively, configure icons in voxtype's config.toml:\n");
    println!("   [status]");
    println!("   icon_theme = \"nerd-font\"");
    println!("\nBuilt-in themes:");
    println!("  Font-based (require specific fonts):");
    println!("    emoji (default), nerd-font, material, phosphor, codicons, omarchy");
    println!("  Universal (no special fonts):");
    println!("    minimal, dots, arrows, text");
    println!("\nOr specify a path to a custom theme TOML file.\n");

    println!("---");
    println!("\nFor more details, see: https://voxtype.io or docs/WAYBAR.md");
}

/// Generate just the JSON config snippet (for programmatic use)
pub fn get_json_config() -> &'static str {
    r#""custom/voxtype": {
    "exec": "voxtype status --follow --format json",
    "return-type": "json",
    "format": "{}",
    "tooltip": true,
    "on-click": "systemctl --user restart voxtype"
}"#
}

/// Generate just the CSS snippet (for programmatic use)
pub fn get_css_config() -> &'static str {
    r#"#custom-voxtype {
    padding: 0 10px;
}

#custom-voxtype.recording {
    color: #ff5555;
    animation: pulse 1s ease-in-out infinite;
}

#custom-voxtype.transcribing {
    color: #f1fa8c;
}

#custom-voxtype.idle {
    color: #50fa7b;
}

#custom-voxtype.stopped {
    color: #6272a4;
}

@keyframes pulse {
    0% {
        opacity: 1;
    }
    50% {
        opacity: 0.5;
    }
    100% {
        opacity: 1;
    }
}"#
}
