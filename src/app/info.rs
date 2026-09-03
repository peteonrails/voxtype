//! `voxtype info <subcommand>` — read-only introspection of the install:
//! binary variants, audio devices, model catalogs, and compiled engines.
//!
//! The `--json` forms exist so a settings UI can populate its pickers without
//! scraping human-readable output. `info devices`, `info models`, and
//! `info styles` back the `dynamic_enum` key types in
//! `voxtype config schema --json`.

use voxtype::audio::devices;
use voxtype::config::Config;
use voxtype::config_set::{engine_feature_compiled, ENGINE_NAMES};
use voxtype::model_catalog::{self, ModelHealth, ModelVerification};
use voxtype::{setup, InfoAction};

/// Dispatch `voxtype info <subcommand>`. `config` is the already-resolved
/// configuration, so `info engines` marks the engine that would actually run
/// right now — honoring `--config` and the `VOXTYPE_ENGINE` override rather
/// than re-reading the default path.
pub(crate) fn run_info_command(action: InfoAction, config: &Config) -> anyhow::Result<()> {
    match action {
        InfoAction::Variants { json } => {
            let inv = setup::binary::inventory();
            if json {
                println!("{}", serde_json::to_string_pretty(&inv)?);
            } else {
                print_variants_text(&inv);
            }
        }
        InfoAction::Devices { json } => run_devices(json)?,
        InfoAction::Models {
            json,
            engine,
            verify,
        } => run_models(json, engine.as_deref(), verify)?,
        InfoAction::Accel { json } => run_accel(json)?,
        InfoAction::Engines { json } => run_engines(json, config)?,
        InfoAction::Styles { json } => run_styles(json, config)?,
    }
    Ok(())
}

fn run_accel(json: bool) -> anyhow::Result<()> {
    let report = setup::accel::report();
    if json {
        println!("{}", serde_json::to_string_pretty(&report.to_json())?);
        return Ok(());
    }

    println!("Acceleration");
    println!("  State:    {}", report.state.tag());
    println!("  Backend:  {}", report.backend.unwrap_or("(none in play)"));
    println!(
        "  Variant:  {}",
        report
            .variant
            .map(|v| v.binary_name().trim_start_matches("voxtype-").to_string())
            .unwrap_or_else(|| "(unrecognized binary)".to_string())
    );
    if let Some(pid) = report.pid {
        println!("  Daemon:   pid {}", pid);
    }
    println!();
    println!("{}", report.state.explanation());

    if !report.evidence.is_empty() {
        println!();
        println!("Evidence");
        for line in &report.evidence {
            println!("  {}", line);
        }
    }
    Ok(())
}

fn run_devices(json: bool) -> anyhow::Result<()> {
    let found = devices::input_devices();
    if json {
        println!("{}", serde_json::to_string_pretty(&found)?);
        return Ok(());
    }
    println!("Audio input devices");
    for d in &found {
        let mark = if d.default { " (default)" } else { "" };
        println!("  {}{}", d.name, mark);
    }
    println!();
    println!("Select one with: voxtype config set audio.device <NAME>");
    Ok(())
}

/// One model's line in the listing, with whatever the integrity checks found.
struct ModelStatus {
    name: &'static str,
    health: ModelHealth,
    /// The `--model` value that fetches this entry, when one exists.
    download_arg: Option<String>,
    /// Only populated with `--verify`.
    verification: Option<ModelVerification>,
}

impl ModelStatus {
    fn check(engine: &str, name: &'static str, verify: bool) -> Self {
        let health = model_catalog::model_health(engine, name);
        // Hashing a model that isn't there has nothing to say.
        let verification = match (verify, &health) {
            (true, ModelHealth::Missing) => None,
            (true, _) => Some(model_catalog::verify_model(engine, name)),
            (false, _) => None,
        };
        Self {
            name,
            health,
            download_arg: model_catalog::download_arg(engine, name),
            verification,
        }
    }

    fn installed(&self) -> bool {
        self.health == ModelHealth::Present
            && !matches!(self.verification, Some(ModelVerification::Corrupt(_)))
    }

    /// Everything wrong with this model, from both tiers of checking.
    fn problems(&self) -> Vec<String> {
        let mut out = match &self.health {
            ModelHealth::Corrupt(p) => p.clone(),
            _ => Vec::new(),
        };
        if let Some(ModelVerification::Corrupt(p)) = &self.verification {
            for problem in p {
                if !out.contains(problem) {
                    out.push(problem.clone());
                }
            }
        }
        out
    }

    fn corrupt(&self) -> bool {
        !self.problems().is_empty()
    }

    /// The `verified` field in JSON: what hashing established, if it ran.
    fn verified_tag(&self) -> Option<&'static str> {
        match &self.verification {
            None => None,
            Some(ModelVerification::Ok) => Some("ok"),
            Some(ModelVerification::Corrupt(_)) => Some("corrupt"),
            Some(ModelVerification::Unverifiable(_)) => Some("unverifiable"),
        }
    }

    fn unverifiable_reason(&self) -> Option<&'static str> {
        match &self.verification {
            Some(ModelVerification::Unverifiable(why)) => Some(why),
            _ => None,
        }
    }

    fn json(&self) -> serde_json::Value {
        let mut o = serde_json::Map::new();
        o.insert("name".into(), serde_json::json!(self.name));
        o.insert("installed".into(), serde_json::json!(self.installed()));
        // `name` is the config value; the argument that downloads it is not
        // always the same string, and for five engines there is no such
        // argument at all. Spelling both out keeps a UI from building
        // `--model <name>` and getting a different model or an error.
        o.insert(
            "downloadable".into(),
            serde_json::json!(self.download_arg.is_some()),
        );
        o.insert("download_arg".into(), serde_json::json!(self.download_arg));
        if self.corrupt() {
            o.insert("corrupt".into(), serde_json::json!(true));
            o.insert("problems".into(), serde_json::json!(self.problems()));
        }
        if let Some(tag) = self.verified_tag() {
            o.insert("verified".into(), serde_json::json!(tag));
        }
        if let Some(why) = self.unverifiable_reason() {
            o.insert("unverifiable_reason".into(), serde_json::json!(why));
        }
        serde_json::Value::Object(o)
    }

    /// Left-hand status column, padded so the names line up.
    ///
    /// Under `--verify`, a model that could not be hashed says so rather than
    /// borrowing the word "installed" from the cheap check.
    fn label(&self) -> &'static str {
        match (&self.health, self.verified_tag()) {
            (ModelHealth::Missing, _) => "         ",
            (_, _) if self.corrupt() => "corrupt  ",
            (_, Some("ok")) => "verified ",
            (_, Some("unverifiable")) => "unchecked",
            _ => "installed",
        }
    }
}

fn run_models(json: bool, only: Option<&str>, verify: bool) -> anyhow::Result<()> {
    let engines: Vec<&str> = match only {
        Some(name) => {
            if !model_catalog::CATALOG_ENGINES.contains(&name) {
                eprintln!(
                    "error: unknown engine '{}'. Valid engines: {}",
                    name,
                    model_catalog::CATALOG_ENGINES.join(", ")
                );
                std::process::exit(2);
            }
            vec![name]
        }
        None => model_catalog::CATALOG_ENGINES.to_vec(),
    };

    let checked: Vec<(&str, Vec<ModelStatus>)> = engines
        .iter()
        .map(|engine| {
            let statuses = model_catalog::model_catalog(engine)
                .into_iter()
                .map(|name| ModelStatus::check(engine, name, verify))
                .collect();
            (*engine, statuses)
        })
        .collect();

    if json {
        let mut map = serde_json::Map::new();
        for (engine, statuses) in &checked {
            let models: Vec<serde_json::Value> = statuses.iter().map(ModelStatus::json).collect();
            map.insert(
                engine.to_string(),
                serde_json::json!({
                    "models": models,
                    "default": model_catalog::default_model(engine),
                }),
            );
        }
        let doc = serde_json::json!({
            "engines": serde_json::Value::Object(map),
            "verified": verify,
        });
        println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(());
    }

    println!("Model catalog  ({})", Config::models_dir().display());
    let mut any_corrupt = false;
    for (engine, statuses) in &checked {
        println!();
        // `setup --download --model` only routes whisper, parakeet and
        // sensevoice names; say so instead of printing a footer that suggests
        // it works everywhere.
        if statuses.iter().all(|s| s.download_arg.is_none()) {
            println!("{}  (download via: voxtype setup model)", engine);
        } else {
            println!("{}", engine);
        }
        let default = model_catalog::default_model(engine);
        for status in statuses {
            let star = if status.name == default {
                " (default)"
            } else {
                ""
            };
            // SenseVoice's config values collide with whisper's model names,
            // so its download argument is the directory form. Show it rather
            // than let someone infer `--model small` and get whisper.
            let arg = match &status.download_arg {
                Some(arg) if arg != status.name => format!("  (download: {})", arg),
                _ => String::new(),
            };
            println!("  {}  {}{}{}", status.label(), status.name, star, arg);
            for problem in status.problems() {
                any_corrupt = true;
                println!("               {}", problem);
            }
        }
    }
    // Say once, rather than per model, why anything marked "unchecked" could
    // not be hashed.
    let mut reasons: Vec<&str> = Vec::new();
    for (_, statuses) in &checked {
        for reason in statuses.iter().filter_map(ModelStatus::unverifiable_reason) {
            if !reasons.contains(&reason) {
                reasons.push(reason);
            }
        }
    }
    if !reasons.is_empty() {
        println!();
        println!("unchecked:");
        for reason in reasons {
            println!("  {}", reason);
        }
    }

    println!();
    if any_corrupt {
        println!("Re-download a damaged model with: voxtype setup --download --model <NAME>");
    } else {
        println!("Download one with: voxtype setup --download --model <NAME>");
    }
    Ok(())
}

fn run_engines(json: bool, config: &Config) -> anyhow::Result<()> {
    let active = config.engine.name();

    if json {
        let list: Vec<serde_json::Value> = ENGINE_NAMES
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "compiled": engine_feature_compiled(name),
                    "active": *name == active,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }

    println!("Transcription engines");
    for name in ENGINE_NAMES {
        let compiled = if engine_feature_compiled(name) {
            "compiled"
        } else {
            "        "
        };
        let mark = if *name == active { " ● active" } else { "" };
        println!("  {}  {}{}", compiled, name, mark);
    }
    println!();
    println!("Switch with: voxtype config set engine <NAME>");
    Ok(())
}

fn run_styles(json: bool, config: &Config) -> anyhow::Result<()> {
    let styles = voxtype::osd::style::list_installed_styles();
    let active = config.osd.style.as_str();

    if json {
        let list: Vec<serde_json::Value> = styles
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "dir": s.dir,
                    "description": s.description,
                    "active": s.name == active,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }

    println!("OSD styles (Quickshell frontend)");
    for s in &styles {
        let mark = if s.name == active { " ● active" } else { "" };
        println!("  {}{}", s.name, mark);
        if let Some(dir) = &s.dir {
            println!("      {}", dir.display());
        }
        if let Some(desc) = &s.description {
            println!("      {}", desc);
        }
    }
    if let Some(path) = &config.osd.plugin_path {
        println!();
        println!(
            "Note: [osd] plugin_path = {} overrides the style selection.",
            path.display()
        );
    }
    println!();
    println!("Switch with: voxtype config set osd.style <NAME>");
    println!("Recipe presets to copy from: /usr/share/voxtype/osd-recipes (or examples/osd-recipes in the source tree)");
    Ok(())
}

fn print_variants_text(inv: &setup::binary::Inventory) {
    use setup::binary::InstallKind;

    println!("Voxtype install");
    println!("  Binary:        {}", inv.binary_path.display());
    println!(
        "  Install kind:  {}",
        match inv.install_kind {
            InstallKind::Package => "package",
            InstallKind::Source => "source",
        }
    );
    if let Some(dir) = &inv.package_lib_dir {
        println!("  Lib dir:       {}", dir.display());
    }
    if !inv.compiled_features.is_empty() {
        println!("  Features:      {}", inv.compiled_features.join(", "));
    }

    println!();
    println!("Hardware");
    println!(
        "  CPU:           AVX2={}, AVX-512={}",
        inv.cpu.avx2, inv.cpu.avx512
    );
    println!(
        "  GPU:           NVIDIA={}, AMD={}",
        inv.gpus.nvidia, inv.gpus.amd
    );

    println!();
    println!("Recommended for this hardware");
    println!(
        "  Whisper:       ★ {}  — {}",
        inv.recommendation.whisper.display(),
        inv.recommendation.whisper_reason
    );
    println!(
        "  ONNX:          ★ {}  — {}",
        inv.recommendation.onnx.display(),
        inv.recommendation.onnx_reason
    );

    println!();
    if matches!(inv.install_kind, InstallKind::Source) {
        println!("Source build: variant switching not applicable.");
        println!("To enable a different engine, rebuild with the appropriate Cargo features.");
        return;
    }

    println!("Variants");
    // Two different facts, previously conflated under "Active": what the
    // daemon is executing right now, and what /usr/bin/voxtype would launch
    // next. They diverge after a variant switch with no restart, or when
    // something other than the symlink started the daemon.
    match (inv.running_variant, inv.daemon_pid) {
        (Some(running), Some(pid)) => {
            println!(
                "  Running:       {} ({})  — daemon pid {}",
                running.display(),
                running.binary_name(),
                pid
            );
        }
        (None, Some(pid)) => {
            println!(
                "  Running:       not a packaged variant  — daemon pid {}",
                pid
            );
        }
        _ => {
            println!("  Running:       no daemon");
        }
    }

    match inv.active_variant {
        Some(next) => println!(
            "  Next launch:   {} ({})",
            next.display(),
            next.binary_name()
        ),
        None => println!("  Next launch:   unknown (symlink missing or unrecognized)"),
    }

    if let (Some(running), Some(next)) = (inv.running_variant, inv.active_variant) {
        if running != next {
            println!();
            println!("  The running daemon and the symlink disagree.");
            println!("  Restart voxtype to pick up {}:", next.display());
            println!("    systemctl --user restart voxtype");
        }
    }

    println!();
    println!("  Available:");
    for status in &inv.variants {
        let mark = if status.active {
            "● active"
        } else if !status.installed {
            "  not installed"
        } else if !status.runs_on_this_cpu {
            "  installed (won't run on this CPU)"
        } else if !status.gpu_available {
            "  installed (no compatible GPU detected)"
        } else {
            "  installed"
        };
        println!("    {:<22} {}", status.variant.display(), mark);
    }
}
