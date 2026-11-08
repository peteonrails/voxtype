//! Exercise the same CLI/schema boundary used by graphical settings clients.
#![cfg(all(feature = "cohere-gguf", feature = "parakeet-gguf"))]

use std::path::Path;
use std::process::Command;

const CASES: [(&str, &str, &str); 2] = [
    (
        "cohere",
        "cohere-transcribe-03-2026-Q4_K_M.gguf",
        "cohere-transcribe-q4f16",
    ),
    (
        "parakeet",
        "parakeet-tdt-0.6b-v3-Q8_0.gguf",
        "/models/parakeet-migraphx",
    ),
];

fn invoke(dir: &Path, args: &[&str]) -> std::process::Output {
    use std::os::unix::fs::PermissionsExt;
    let worker = dir.join("worker-fixture");
    std::fs::write(&worker, "#!/bin/sh\nexit 99\n").unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("VOXTYPE_") {
            cmd.env_remove(key);
        }
    }
    cmd.env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_RUNTIME_DIR", dir.join("runtime"))
        .env("VOXTYPE_GGUF_WORKER", worker)
        .arg("--config")
        .arg(dir.join("config.toml"))
        .args(args)
        .output()
        .unwrap()
}

fn cli(dir: &Path, args: &[&str]) -> String {
    let result = invoke(dir, args);
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn fresh_cli_selection_and_schema_agree_on_gguf_defaults() {
    for (engine, expected, _) in CASES {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "# Fresh config\n").unwrap();
        let models: serde_json::Value =
            serde_json::from_str(&cli(dir.path(), &["info", "models", "--json"])).unwrap();
        assert_eq!(models["engines"][engine]["default"], expected);
        let schema: serde_json::Value =
            serde_json::from_str(&cli(dir.path(), &["config", "schema", "--json"])).unwrap();
        let key = format!("{engine}.model");
        let field = schema["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["key"] == key)
            .unwrap();
        assert_eq!(field["value"], expected);
        assert_eq!(field["compiled"], true);
        // Sparse size/header fixture is only for persistence, never inference.
        let store = dir.path().join("data/voxtype/models");
        std::fs::create_dir_all(&store).unwrap();
        let mut file = std::fs::File::create(store.join(expected)).unwrap();
        file.set_len(if engine == "cohere" {
            1_558_162_944
        } else {
            739_508_576
        })
        .unwrap();
        use std::io::Write;
        file.write_all(b"GGUF").unwrap();
        cli(dir.path(), &["config", "set", "engine", engine]);
        let source = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let saved: toml::Value = toml::from_str(&source).unwrap();
        assert_eq!(saved["engine"].as_str(), Some(engine));
        assert_eq!(saved[engine]["model"].as_str(), Some(expected));
        assert_eq!(cli(dir.path(), &["config", "get", &key]).trim(), expected);
        // Setting a sibling through the schema must not replace the model.
        cli(
            dir.path(),
            &["config", "set", &format!("{engine}.threads"), "2"],
        );
        assert_eq!(cli(dir.path(), &["config", "get", &key]).trim(), expected);
    }
}

#[test]
fn legacy_cli_selection_and_schema_do_not_migrate_explicit_models() {
    for (engine, _, model) in CASES {
        let dir = tempfile::tempdir().unwrap();
        let source = format!("# Explicit legacy configuration\nengine = \"{engine}\"\n[{engine}]\nmodel = \"{model}\" # pinned\nthreads = 6\non_demand_loading = true\n");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, &source).unwrap();
        let result = invoke(dir.path(), &["config", "set", "engine", engine]);
        assert!(
            !result.status.success(),
            "uninstalled or unsupported legacy model must not be committed"
        );
        let schema: serde_json::Value =
            serde_json::from_str(&cli(dir.path(), &["config", "schema", "--json"])).unwrap();
        let key = format!("{engine}.model");
        let field = schema["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["key"] == key)
            .unwrap();
        assert_eq!(field["value"], model);
        assert_eq!(cli(dir.path(), &["config", "get", &key]).trim(), model);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    }
}
