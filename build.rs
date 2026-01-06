//! Build script for voxtype
//!
//! Generates man pages from CLI definitions using clap_mangen.

use clap::CommandFactory;
use clap_mangen::Man;
use std::env;
use std::fs::{self, File};
use std::io::Error;
use std::path::PathBuf;

// Include the CLI module
include!("src/cli.rs");

fn main() -> Result<(), Error> {
    // Only generate man pages for release builds or when explicitly requested
    let profile = env::var("PROFILE").unwrap_or_default();
    let generate = env::var("VOXTYPE_GEN_MANPAGES").is_ok() || profile == "release";

    if !generate {
        return Ok(());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_else(|_| "target".to_string()));
    let man_dir = out_dir.join("man");
    fs::create_dir_all(&man_dir)?;

    let cmd = Cli::command();

    // Generate main man page (voxtype.1)
    let man = Man::new(cmd.clone());
    let mut file = File::create(man_dir.join("voxtype.1"))?;
    man.render(&mut file)?;

    // Generate man pages for subcommands
    for subcommand in cmd.get_subcommands() {
        let name = subcommand.get_name();
        if name == "help" {
            continue;
        }

        let man = Man::new(subcommand.clone());
        let mut file = File::create(man_dir.join(format!("voxtype-{}.1", name)))?;
        man.render(&mut file)?;

        // Generate man pages for nested subcommands (e.g., voxtype-setup-gpu)
        for nested in subcommand.get_subcommands() {
            let nested_name = nested.get_name();
            if nested_name == "help" {
                continue;
            }

            let man = Man::new(nested.clone());
            let mut file =
                File::create(man_dir.join(format!("voxtype-{}-{}.1", name, nested_name)))?;
            man.render(&mut file)?;
        }
    }

    // Tell cargo to rerun if CLI definitions change
    println!("cargo:rerun-if-changed=src/cli.rs");

    // Print location of generated man pages
    println!(
        "cargo:warning=Man pages generated in: {}",
        man_dir.display()
    );

    Ok(())
}
