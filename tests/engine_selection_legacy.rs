//! Persistence-only legacy-capable build fixture, not an ONNX inference claim.
use voxtype::config_set::set_engine;

#[test]
fn working_explicit_onnx_format_and_manual_paths_are_preserved() {
    for (engine, compiled) in [
        ("parakeet", cfg!(feature = "parakeet")),
        ("cohere", cfg!(feature = "cohere")),
    ] {
        if !compiled {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join("custom-migraphx-model");
        std::fs::create_dir(&model).unwrap();
        // Custom checkpoints have no catalog manifest. These files document the
        // fixture's intended format; no fake weights are passed to inference.
        std::fs::write(model.join("encoder-model.onnx"), b"selection fixture").unwrap();
        std::fs::write(model.join("vocab.txt"), b"selection fixture").unwrap();
        let path = directory.path().join("config.toml");
        let source = format!("# Keep my legacy settings\nengine = \"{engine}\" # engine\n[{engine}]\nmodel = {:?} # manual path\nthreads = 6\non_demand_loading = false\nexecution_provider = \"migraphx\"\n[custom]\nkeep = \"unchanged\"\n", model.to_str().unwrap());
        std::fs::write(&path, &source).unwrap();
        set_engine(path.clone(), engine).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        let config = voxtype::config::load_config(Some(&path)).unwrap();
        assert_eq!(config.model_name(), model.to_str().unwrap());
    }
}
