//! The manifest of an export: every file with its BLAKE3 hash and size, the
//! case (pseudonymised code), the settings and the versions, as JSON.
//!
//! The manifest is written last and lists every file of the folder except
//! itself. [`verify`] reads a folder back and compares every file with its
//! entry, which is how a lab or the clinic checks that the package arrived
//! whole and unchanged.

use std::path::Path;

use dc_case::BlobRef;
use serde::{Deserialize, Serialize};

/// Name of the manifest file.
pub const MANIFEST_NAME: &str = "manifest.json";
/// Version of the manifest format.
pub const FORMAT_VERSION: u32 = 1;

/// BLAKE3 of `bytes`, 64 lowercase hex digits.
pub fn hash(bytes: &[u8]) -> String {
    BlobRef::of_bytes(bytes).to_hex()
}

/// One file of the export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Path relative to the export folder, with `/` separators.
    pub name: String,
    /// What it is: `stl`, `csv`, `ics`, `html` or `json`.
    pub kind: String,
    /// Size in bytes.
    pub bytes: u64,
    /// BLAKE3 of the content.
    pub blake3: String,
    /// `upper` or `lower` for the files of one arch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    /// Stage of a model or aligner (`0`: initial situation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<u32>,
    /// Print check of a mesh: `true` when it passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printable: Option<bool>,
}

impl Entry {
    /// The entry of `bytes` stored as `name`.
    pub fn of(name: &str, kind: &str, bytes: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            kind: kind.to_owned(),
            bytes: bytes.len() as u64,
            blake3: hash(bytes),
            arch: None,
            stage: None,
            printable: None,
        }
    }
}

/// The software that made the export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generator {
    /// Name of the module.
    pub name: String,
    /// Version of the algorithms crate.
    pub version: String,
}

/// Summary of the print check.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PrintSummary {
    /// Meshes checked.
    pub checked: usize,
    /// Meshes that pass.
    pub passed: usize,
    /// Names of the ones that do not.
    pub failed: Vec<String>,
}

/// The manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Version of this format.
    pub format_version: u32,
    /// The software.
    pub generator: Generator,
    /// Pseudonymised patient code of the case.
    pub patient_code: String,
    /// Date of the export, `YYYY-MM-DD`.
    pub generated: String,
    /// The settings of the export.
    pub settings: serde_json::Value,
    /// Active aligners of each arch.
    pub aligners: std::collections::BTreeMap<String, u32>,
    /// Aligners of the longest series.
    pub total_aligners: u32,
    /// The files, by name.
    pub files: Vec<Entry>,
    /// The print check.
    pub print_check: PrintSummary,
}

/// What [`verify`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Verification {
    /// Files listed in the manifest.
    pub listed: usize,
    /// Files whose size and hash match.
    pub verified: usize,
    /// Listed files that are not in the folder.
    pub missing: Vec<String>,
    /// Files whose size or hash differ.
    pub mismatched: Vec<String>,
    /// Files in the folder that the manifest does not list.
    pub unlisted: Vec<String>,
}

impl Verification {
    /// Every file is there and unchanged.
    pub fn ok(&self) -> bool {
        self.missing.is_empty() && self.mismatched.is_empty() && self.unlisted.is_empty()
    }
}

/// Errors of the manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The manifest or a file could not be read.
    #[error("{path}: {message}")]
    Io {
        /// File.
        path: String,
        /// What happened.
        message: String,
    },
    /// The manifest is not valid.
    #[error("{MANIFEST_NAME}: {0}")]
    Invalid(String),
}

fn io(path: &Path, e: impl std::fmt::Display) -> ManifestError {
    ManifestError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    }
}

/// Reads the manifest of the folder `dir`.
pub fn read(dir: &Path) -> Result<Manifest, ManifestError> {
    let path = dir.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&path).map_err(|e| io(&path, e))?;
    serde_json::from_str(&text).map_err(|e| ManifestError::Invalid(e.to_string()))
}

/// Every file under `dir` (relative names, `/` separators, sorted), the
/// manifest and the temporary files excluded.
pub fn list_files(dir: &Path) -> Result<Vec<String>, ManifestError> {
    fn walk(root: &Path, at: &Path, out: &mut Vec<String>) -> Result<(), ManifestError> {
        for e in std::fs::read_dir(at).map_err(|e| io(at, e))? {
            let path = e.map_err(|e| io(at, e))?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if let Ok(rel) = path.strip_prefix(root) {
                let name = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if name != MANIFEST_NAME && !name.ends_with(".tmp") {
                    out.push(name);
                }
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// The kind of a file from its extension.
pub fn kind_of(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "stl" => "stl",
        "csv" => "csv",
        "ics" => "ics",
        "html" => "html",
        "json" => "json",
        _ => "file",
    }
}

/// The entries of the files found under `dir` (hash and size read from
/// disk): the manifest of a folder that has none.
pub fn scan(dir: &Path) -> Result<Vec<Entry>, ManifestError> {
    list_files(dir)?
        .into_iter()
        .map(|name| {
            let path = dir.join(&name);
            let bytes = std::fs::read(&path).map_err(|e| io(&path, e))?;
            Ok(Entry::of(&name, kind_of(&name), &bytes))
        })
        .collect()
}

/// Compares the folder `dir` with its manifest.
pub fn verify(dir: &Path) -> Result<Verification, ManifestError> {
    let manifest = read(dir)?;
    let mut v = Verification {
        listed: manifest.files.len(),
        ..Verification::default()
    };
    for e in &manifest.files {
        let path = dir.join(&e.name);
        match std::fs::read(&path) {
            Err(_) => v.missing.push(e.name.clone()),
            Ok(bytes) if bytes.len() as u64 == e.bytes && hash(&bytes) == e.blake3 => {
                v.verified += 1
            }
            Ok(_) => v.mismatched.push(e.name.clone()),
        }
    }
    let listed: std::collections::BTreeSet<&str> =
        manifest.files.iter().map(|e| e.name.as_str()).collect();
    v.unlisted = list_files(dir)?
        .into_iter()
        .filter(|n| !listed.contains(n.as_str()))
        .collect();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("dca-manifest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample(dir: &Path) -> Manifest {
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.csv"), b"x,y\n1,2\n").unwrap();
        std::fs::write(dir.join("sub/b.stl"), [1u8, 2, 3]).unwrap();
        Manifest {
            format_version: FORMAT_VERSION,
            generator: Generator {
                name: "t".into(),
                version: "0".into(),
            },
            patient_code: "P-1".into(),
            generated: "2026-01-01".into(),
            settings: serde_json::json!({}),
            aligners: Default::default(),
            total_aligners: 0,
            files: scan(dir).unwrap(),
            print_check: PrintSummary::default(),
        }
    }

    #[test]
    fn the_hash_is_blake3() {
        // BLAKE3 of the empty input
        assert_eq!(
            hash(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn a_folder_verifies_against_its_manifest_until_it_changes() {
        let dir = temp("verify");
        let m = sample(&dir);
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.files[1].name, "sub/b.stl");
        assert_eq!(m.files[1].kind, "stl");
        std::fs::write(
            dir.join(MANIFEST_NAME),
            serde_json::to_vec_pretty(&m).unwrap(),
        )
        .unwrap();
        let v = verify(&dir).unwrap();
        assert!(v.ok() && v.verified == 2 && v.listed == 2, "{v:?}");

        std::fs::write(dir.join("a.csv"), b"x,y\n1,3\n").unwrap();
        std::fs::remove_file(dir.join("sub/b.stl")).unwrap();
        std::fs::write(dir.join("extra.txt"), b"?").unwrap();
        let v = verify(&dir).unwrap();
        assert!(!v.ok());
        assert_eq!(v.mismatched, ["a.csv"]);
        assert_eq!(v.missing, ["sub/b.stl"]);
        assert_eq!(v.unlisted, ["extra.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_broken_manifest_is_an_error() {
        let dir = temp("broken");
        assert!(matches!(verify(&dir), Err(ManifestError::Io { .. })));
        std::fs::write(dir.join(MANIFEST_NAME), b"{").unwrap();
        assert!(matches!(verify(&dir), Err(ManifestError::Invalid(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
