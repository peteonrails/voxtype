//! Man page rendering, shared by `build.rs` and `cargo xtask man`.
//!
//! `build.rs` includes this file via `#[path]` so release builds keep
//! producing man pages in `OUT_DIR`, while `cargo xtask man` renders the
//! same pages without compiling voxtype itself. Packaging uses the xtask
//! route: the CI packaging job has no `target/release`, which is how
//! v1.1.1's deb and rpm shipped without man pages.

use clap::Command;
use clap_mangen::Man;
use std::fs::{self, File};
use std::io::Error;
use std::path::Path;

/// Render `voxtype.1`, one page per subcommand (`voxtype-<sub>.1`) and one
/// per nested subcommand (`voxtype-<sub>-<nested>.1`) into `man_dir`.
/// Returns the number of pages written.
pub fn write_man_pages(cmd: &Command, man_dir: &Path) -> Result<usize, Error> {
    fs::create_dir_all(man_dir)?;
    let mut written = 0;

    let mut file = File::create(man_dir.join("voxtype.1"))?;
    Man::new(cmd.clone()).render(&mut file)?;
    written += 1;

    for subcommand in cmd.get_subcommands() {
        let name = subcommand.get_name();
        if name == "help" {
            continue;
        }

        let mut file = File::create(man_dir.join(format!("voxtype-{}.1", name)))?;
        Man::new(subcommand.clone()).render(&mut file)?;
        written += 1;

        for nested in subcommand.get_subcommands() {
            let nested_name = nested.get_name();
            if nested_name == "help" {
                continue;
            }

            let mut file =
                File::create(man_dir.join(format!("voxtype-{}-{}.1", name, nested_name)))?;
            Man::new(nested.clone()).render(&mut file)?;
            written += 1;
        }
    }

    Ok(written)
}
