//! Download and inspect the optional punctuation/casing cleanup model.

use super::{model, print_info, print_success, print_warning};

pub fn download_model() -> anyhow::Result<()> {
    let path = model::punctuation_model_path();
    if path.join("model.onnx").exists() && path.join("tokenizer.json").exists() {
        print_success(&format!(
            "Punctuation/casing model already installed: {}",
            path.display()
        ));
        return Ok(());
    }

    println!("Downloading punctuation/casing model (about 1.1 GB)...");
    model::download_punctuation_model()?;
    print_success("Punctuation/casing model installed.");
    print_info("Enable with: voxtype config set text.punctuation_casing true");
    Ok(())
}

pub fn show_status() {
    let path = model::punctuation_model_path();
    if path.join("model.onnx").exists() && path.join("tokenizer.json").exists() {
        print_success(&format!(
            "Punctuation/casing model installed: {}",
            path.display()
        ));
    } else {
        print_warning("Punctuation/casing model is not installed");
        print_info("Download with: voxtype setup punctuation");
    }
}
