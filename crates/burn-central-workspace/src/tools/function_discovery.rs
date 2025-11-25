//! Build-time function discovery using cargo rustc macro expansion
//!
//! Uses `cargo rustc -- -Zunpretty=expanded` to extract `BCFN1|mod_path|fn|builder|routine|proc_type|END` markers from the expanded source code.

use serde::{Deserialize, Serialize};

use quote::ToTokens;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const MAGIC: &str = "BCFN1|";
const END: &str = "|END";
const SEP: char = '|';

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FunctionMetadata {
    pub mod_path: String,
    pub fn_name: String,
    pub builder_fn_name: String,
    pub routine_name: String,
    pub proc_type: String,
    pub token_stream: Vec<u8>,
}

impl FunctionMetadata {
    pub fn get_function_code(&self) -> String {
        if self.token_stream.is_empty() {
            // If no token stream is available, create a placeholder function
            format!(
                "fn {}() {{\n    // Function implementation not available\n}}",
                self.fn_name
            )
        } else {
            match syn_serde::json::from_slice::<syn::ItemFn>(&self.token_stream) {
                Ok(itemfn) => match syn::parse2(itemfn.into_token_stream()) {
                    Ok(syn_tree) => prettyplease::unparse(&syn_tree),
                    Err(_) => format!(
                        "fn {}() {{\n    // Failed to parse token stream\n}}",
                        self.fn_name
                    ),
                },
                Err(_) => format!(
                    "fn {}() {{\n    // Failed to deserialize token stream\n}}",
                    self.fn_name
                ),
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct FunctionDiscovery {
    project_root: PathBuf,
    target_dir: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
}

impl FunctionDiscovery {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            target_dir: None,
            manifest_path: None,
        }
    }

    pub fn with_target_dir(mut self, target_dir: impl Into<PathBuf>) -> Self {
        self.target_dir = Some(target_dir.into());
        self
    }

    pub fn with_manifest_path(mut self, manifest_path: impl Into<PathBuf>) -> Self {
        self.manifest_path = Some(manifest_path.into());
        self
    }

    /// Expand and extract in one call.
    pub fn discover_functions(&self) -> Result<Vec<FunctionMetadata>, String> {
        let expanded = self.expand_with_cargo()?;
        let functions = parse_expanded_output(&expanded);
        Ok(functions)
    }

    fn expand_with_cargo(&self) -> Result<String, String> {
        let mut cmd = Command::new("cargo");
        cmd.current_dir(&self.project_root)
            .arg("rustc")
            .arg("--lib")
            .arg("--profile=check");

        if let Some(mp) = &self.manifest_path {
            cmd.arg("--manifest-path").arg(mp);
        }
        if let Some(td) = &self.target_dir {
            cmd.arg("--target-dir").arg(td);
        }

        cmd.arg("--");

        cmd.arg("-Zunpretty=expanded");

        cmd.env("RUSTC_BOOTSTRAP", "1");
        cmd.env("RUST_LOG", "error");

        // We want to read both streams.
        let child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn cargo rustc: {e}"))?;

        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("cargo rustc failed (status {})", output.status));
        }
        let expanded = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;

        Ok(expanded)
    }
}

pub fn parse_expanded_output(expanded: &str) -> Vec<FunctionMetadata> {
    let bytes = expanded.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();

    while let Some(m) = find(bytes, MAGIC.as_bytes(), i) {
        let start_payload = m + MAGIC.len();
        if let Some(end) = find(bytes, END.as_bytes(), start_payload) {
            if let Ok(slice) = std::str::from_utf8(&bytes[m..end + END.len()]) {
                if let Some(meta) = parse_bcfn_marker(slice) {
                    out.push(meta);
                }
            }
            i = end + END.len();
        } else {
            // no closing sentinel; stop scanning
            break;
        }
    }
    out
}

/// Expected `BCFN1|mod_path|fn_name|builder|routine|proc_type|END`.
fn parse_bcfn_marker(marker: &str) -> Option<FunctionMetadata> {
    if !marker.starts_with(MAGIC) || !marker.ends_with(END) {
        return None;
    }
    let body = &marker[MAGIC.len()..marker.len() - END.len()];
    let mut it = body.split(SEP);

    let mod_path = it.next()?.to_string();
    let fn_name = it.next()?.to_string();
    let builder_fn_name = it.next()?.to_string();
    let routine_name = it.next()?.to_string();
    let proc_type = it.next()?.to_string();

    // There must be exactly 5 parts.
    if it.next().is_some() {
        return None;
    }

    Some(FunctionMetadata {
        mod_path,
        fn_name,
        builder_fn_name,
        routine_name,
        proc_type,
        token_stream: Vec::new(),
    })
}

/// Naive byte-substring search (no regex).
fn find(hay: &[u8], needle: &[u8], mut from: usize) -> Option<usize> {
    while from + needle.len() <= hay.len() {
        if &hay[from..from + needle.len()] == needle {
            return Some(from);
        }
        from += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_markers() {
        let expanded = r#"
            /* noise */ const X:&str="hello";
            const BURN_CENTRAL_FUNCTION_TRAIN:&str="BCFN1|my::module|train_fn|__train_fn_builder|train|training|END";
            const BURN_CENTRAL_FUNCTION_EVAL:&str=
                "BCFN1|my::module|eval_fn|__eval_fn_builder|evaluate|training|END";
        "#;

        let v = parse_expanded_output(expanded);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].mod_path, "my::module");
        assert_eq!(v[0].fn_name, "train_fn");
        assert_eq!(v[1].fn_name, "eval_fn");
        assert_eq!(v[1].routine_name, "evaluate");
    }

    #[test]
    fn rejects_bad_marker() {
        let bad = "BCFN1|a|b|c|d|END"; // missing one field
        assert!(parse_bcfn_marker(bad).is_none());
    }

    #[test]
    fn accepts_complex_mod_path() {
        let ok = "BCFN1|a::b::c::d|f|__builder|r|training|END";
        let m = parse_bcfn_marker(ok).unwrap();
        assert_eq!(m.mod_path, "a::b::c::d");
    }
}
