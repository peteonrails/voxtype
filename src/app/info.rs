//! `voxtype info <subcommand>` — currently `info variants`, which inspects
//! the installed binary set and prints CPU/GPU recommendations.

use voxtype::{setup, InfoAction};

/// Dispatch `voxtype info <subcommand>`.
pub(crate) fn run_info_command(action: InfoAction) -> anyhow::Result<()> {
    match action {
        InfoAction::Variants { json } => {
            let inv = setup::binary::inventory();
            if json {
                println!("{}", serde_json::to_string_pretty(&inv)?);
            } else {
                print_variants_text(&inv);
            }
        }
    }
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
    if let Some(active) = inv.active_variant {
        println!(
            "  Active:        {} ({})",
            active.display(),
            active.binary_name()
        );
    } else {
        println!("  Active:        unknown (symlink missing or unrecognized)");
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
