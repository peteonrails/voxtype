//! Verify cleanup overrides using the compiled CLI, not a second parser.
use std::path::Path;
use std::process::Command;

fn get(path: &Path, key: &str, env: (&str, &str), flags: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("VOXTYPE_") {
            cmd.env_remove(name);
        }
    }
    let out = cmd
        .env(env.0, env.1)
        .arg("--config")
        .arg(path)
        .args(flags)
        .args(["config", "get", key])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{key} {flags:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn cleanup_config_env_and_cli_have_independent_parity() {
    let cases = [
        (
            "spoken_punctuation",
            "SPOKEN_PUNCTUATION",
            "spoken-punctuation",
        ),
        ("filter_filler_words", "FILTER_FILLERS", "filter-fillers"),
        (
            "collapse_restarts",
            "COLLAPSE_RESTARTS",
            "collapse-restarts",
        ),
        (
            "repair_sentence_breaks",
            "REPAIR_SENTENCE_BREAKS",
            "repair-sentence-breaks",
        ),
        ("format_numbers", "FORMAT_NUMBERS", "format-numbers"),
        ("lard_disfluency", "LARD_DISFLUENCY", "lard-disfluency"),
        (
            "punctuation_casing",
            "PUNCTUATION_CASING",
            "punctuation-casing",
        ),
        (
            "smart_auto_submit",
            "SMART_AUTO_SUBMIT",
            "smart-auto-submit",
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let source = "# file layer must remain untouched\nengine = 'cohere'\n[cohere]\nmodel = 'explicit-legacy-onnx'\n[text]\nspoken_punctuation = false\nfilter_filler_words = false\ncollapse_restarts = false\nrepair_sentence_breaks = false\nformat_numbers = false\nlard_disfluency = false\npunctuation_casing = false\nsmart_auto_submit = false\nlard_model_path = '/file/model.vxdf'\n";
    std::fs::write(&path, source).unwrap();
    for (field, variable, flag) in cases {
        let key = format!("text.{field}");
        let name = format!("VOXTYPE_{variable}");
        assert_eq!(
            get(&path, &key, (&name, "true"), &[]),
            "true",
            "environment override for {key}"
        );
        assert_eq!(
            get(&path, &key, (&name, "true"), &[&format!("--no-{flag}")]),
            "false",
            "CLI overrides env for {key}"
        );
        assert_eq!(
            get(&path, &key, (&name, "false"), &[&format!("--{flag}")]),
            "true",
            "CLI enable for {key}"
        );
        for other in ["collapse_restarts", "lard_disfluency"] {
            if field != other {
                assert_eq!(
                    get(&path, &format!("text.{other}"), (&name, "true"), &[]),
                    "false",
                    "{field} enabled {other}"
                );
            }
        }
    }
    assert_eq!(
        get(
            &path,
            "text.lard_model_path",
            ("VOXTYPE_LARD_MODEL_PATH", "/env/model.vxdf"),
            &[]
        ),
        "/env/model.vxdf"
    );
    assert_eq!(
        get(
            &path,
            "text.lard_model_path",
            ("VOXTYPE_LARD_MODEL_PATH", "/env/model.vxdf"),
            &["--lard-model-path", "/cli/model.vxdf"]
        ),
        "/cli/model.vxdf"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
}
