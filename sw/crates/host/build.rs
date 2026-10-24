//! Embed the host binary's own git hash (`git_hash` in run records, DEMO.md §4):
//! HEAD at build time, `-dirty` if the host sources or demo/ assets had
//! uncommitted changes. The model's hash is separate (`splitsys_cycle`).

use std::path::Path;
use std::process::Command;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let git = |args: &[&str]| -> Option<String> {
        let o = Command::new("git").arg("-C").arg(&root).args(args).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let hash = match git(&["rev-parse", "--short", "HEAD"]) {
        Some(h) if !h.is_empty() => match git(&["status", "--porcelain", "--", "sw/crates/host", "demo"]) {
            Some(s) if s.is_empty() => h,
            _ => format!("{h}-dirty"),
        },
        _ => "unknown".to_string(),
    };
    println!("cargo:rustc-env=SPLITSYS_HOST_GIT_HASH={hash}");
    for p in [".git/HEAD", ".git/refs/heads", ".git/packed-refs", "demo"] {
        println!("cargo:rerun-if-changed={}", root.join(p).display());
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
}
