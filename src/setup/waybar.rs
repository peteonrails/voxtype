//! Waybar configuration generation for voxtype

/// Generate and print Waybar configuration
pub fn print_config() {
    println!("Waybar Configuration for Voxtype\n");
    println!("================================\n");

    println!("1. Add this to your Waybar config (usually ~/.config/waybar/config):\n");
    println!("   In the \"modules-right\" (or left/center) array, add: \"custom/voxtype\"\n");

    println!("   Then add this module configuration:\n");
    println!(r#"   "custom/voxtype": {{
       "exec": "voxtype status --follow --format json",
       "return-type": "json",
       "format": "{{}}",
       "tooltip": true,
       "on-click": "systemctl --user restart voxtype"
   }}"#);

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
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
}"#
}
