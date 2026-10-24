//! `splitsys_vec`: pure generators for SplitSys test vectors. Every generator returns
//! a list of [`OutFile`]s (path relative to the output directory + exact bytes); the
//! binary writes them. Same seed + same [`Meta`] gives byte-identical output.

pub mod bench;
pub mod jobs;
pub mod selftest;
pub mod unit;

use serde_json::{json, Value};

/// Provenance stamped into every generated file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    /// Short git hash of HEAD when the generator ran ("unknown" outside git).
    pub git_hash: String,
    /// True if the generator's own sources (golden, vec, spec) had uncommitted changes.
    pub git_dirty: bool,
    /// The subcommand line, e.g. `gemm --seed 1 --n 20 --mode full`.
    pub command: String,
}

impl Meta {
    /// Fixed metadata for tests.
    pub fn test() -> Self {
        Self {
            git_hash: "test".into(),
            git_dirty: false,
            command: "test".into(),
        }
    }

    /// JSON metadata block for one file.
    pub fn json(&self, seed: u64, kind: &str) -> Value {
        json!({
            "generator": "splitsys-vec",
            "generator_version": env!("CARGO_PKG_VERSION"),
            "git_hash": self.git_hash,
            "git_dirty": self.git_dirty,
            "command": self.command,
            "seed": seed,
            "kind": kind,
            "spec_version": splitsys_spec::SPEC_VERSION,
        })
    }

    /// Comment header for `$readmemh` files.
    pub fn mem_header(&self, seed: u64, kind: &str, extra: &[String]) -> String {
        let mut s = format!(
            "// splitsys-vec {} kind={} seed={} git={}{} spec={}\n// command: {}\n",
            env!("CARGO_PKG_VERSION"),
            kind,
            seed,
            self.git_hash,
            if self.git_dirty { "-dirty" } else { "" },
            splitsys_spec::SPEC_VERSION,
            self.command
        );
        for e in extra {
            s.push_str("// ");
            s.push_str(e);
            s.push('\n');
        }
        s
    }
}

/// One generated file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutFile {
    /// Path relative to the output directory (forward slashes).
    pub path: String,
    pub bytes: Vec<u8>,
}

impl OutFile {
    pub fn json_pretty(path: impl Into<String>, v: &Value) -> Self {
        let mut s = serde_json::to_string_pretty(v).expect("serialize");
        s.push('\n');
        Self {
            path: path.into(),
            bytes: s.into_bytes(),
        }
    }

    pub fn json_compact(path: impl Into<String>, v: &Value) -> Self {
        let mut s = serde_json::to_string(v).expect("serialize");
        s.push('\n');
        Self {
            path: path.into(),
            bytes: s.into_bytes(),
        }
    }

    pub fn text(path: impl Into<String>, s: String) -> Self {
        Self {
            path: path.into(),
            bytes: s.into_bytes(),
        }
    }
}

/// Lowercase hex of bytes in address order.
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
