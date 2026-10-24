//! Codegen: `cd sw && cargo run -p splitsys-spec --bin gen [-- --check]`.
//!
//! Reads spec/spec.toml, validates it, and writes rtl/gen/splitsys_pkg.sv and
//! sw/crates/spec/src/generated.rs. With `--check`, writes nothing and exits 1
//! if either file is out of date.

use splitsys_spec::model::{rs_out_path, sv_out_path, Spec};
use std::process::ExitCode;

fn main() -> ExitCode {
    let check = std::env::args().skip(1).any(|a| a == "--check");
    let spec = match Spec::load_default() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gen: {e}");
            return ExitCode::FAILURE;
        }
    };
    let outputs = [(sv_out_path(), spec.render_sv()), (rs_out_path(), spec.render_rust())];
    let mut stale = false;
    for (path, text) in &outputs {
        let current = std::fs::read_to_string(path).ok();
        if current.as_deref() == Some(text.as_str()) {
            println!("gen: up to date  {}", path.display());
            continue;
        }
        if check {
            eprintln!("gen: STALE  {}", path.display());
            stale = true;
            continue;
        }
        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("gen: cannot create {}: {e}", dir.display());
                return ExitCode::FAILURE;
            }
        }
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("gen: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("gen: wrote  {}", path.display());
    }
    if stale {
        eprintln!("gen: run `cd sw && cargo run -p splitsys-spec --bin gen` to regenerate");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
