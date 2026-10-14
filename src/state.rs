//! oqman's folders and what it installed (`state.json`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::util;

/// oqman's own folder: sources, build cache, dependencies, a private Rust toolchain and
/// `state.json`. `OQMAN_HOME` overrides it.
pub fn home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("OQMAN_HOME").filter(|h| !h.is_empty()) {
        return Ok(PathBuf::from(h));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).context("LOCALAPPDATA isn't set")?
    } else if cfg!(target_os = "macos") {
        util::home_dir()?.join("Library/Application Support")
    } else {
        match std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
            Some(d) => PathBuf::from(d),
            None => util::home_dir()?.join(".local/share"),
        }
    };
    Ok(base.join("oqman"))
}

/// Folders inside [`home`].
pub struct Dirs {
    /// The root.
    pub home: PathBuf,
}

impl Dirs {
    /// Downloaded archives (kept so a re-install doesn't download again).
    pub fn downloads(&self) -> PathBuf {
        self.home.join("downloads")
    }
    /// Unpacked source trees, one per commit.
    pub fn src(&self) -> PathBuf {
        self.home.join("src")
    }
    /// Cargo's target folder, kept between builds so updates rebuild only what changed.
    pub fn target(&self) -> PathBuf {
        self.home.join("target")
    }
    /// Build dependencies (FFmpeg, LLVM, NASM).
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub fn deps(&self) -> PathBuf {
        self.home.join("deps")
    }
    /// The private Rust toolchain (`rustup` and `cargo` homes), when the system has none.
    pub fn rust(&self) -> PathBuf {
        self.home.join("rust")
    }
    fn state(&self) -> PathBuf {
        self.home.join("state.json")
    }

    /// The saved state, if OpenQ was installed.
    pub fn load(&self) -> Result<Option<State>> {
        match std::fs::read_to_string(self.state()) {
            Ok(text) => Ok(Some(serde_json::from_str(&text).context("state.json is damaged")?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Saves the state.
    pub fn save(&self, state: &State) -> Result<()> {
        std::fs::create_dir_all(&self.home)?;
        let tmp = self.state().with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
        std::fs::rename(&tmp, self.state())?;
        Ok(())
    }

    /// Forgets the installation.
    pub fn clear(&self) -> Result<()> {
        util::remove(&self.state())
    }
}

/// What was installed, and how, so `update` can repeat it.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct State {
    /// The options it was installed with.
    pub options: Options,
    /// The source commit that was built (or "local").
    pub commit: String,
    /// OpenQ's version (from its Cargo.toml).
    pub version: String,
    /// When (Unix seconds).
    pub installed_at: u64,
    /// The oqman version that installed it: `update` redoes the install when oqman has changed since (menu entry,
    /// icons), even if OpenQ hasn't. Empty for installs made before oqman recorded it.
    pub installer: String,
    /// Files and folders the installation created outside oqman's home (app, shortcuts).
    pub installed: Vec<PathBuf>,
}

/// Choices that stay the same across updates.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Options {
    /// GitHub repository, `owner/name`.
    pub repo: String,
    /// A git URL to get the source from with `git` (SSH keys and credential helpers work)
    /// instead of GitHub's API.
    pub git: Option<String>,
    /// A tag, branch or commit to follow; `None` = the newest version tag (or the default
    /// branch while there are no tags).
    pub reference: Option<String>,
    /// A local source tree to build instead of downloading.
    pub source: Option<PathBuf>,
    /// Extra cargo features.
    pub features: Vec<String>,
    /// Build without cargo's default features (no video, no PipeWire), plus `features`.
    pub no_default_features: bool,
    /// macOS: use the FFmpeg found by pkg-config (Homebrew) instead of building an LGPL one.
    pub system_ffmpeg: bool,
    /// Code-signing identity: macOS keychain identity name, or Windows certificate SHA-1
    /// thumbprint. `None` = ad hoc on macOS, unsigned on Windows.
    pub sign: Option<String>,
    /// macOS: notarytool keychain profile to notarise the app with (needs `sign`).
    pub notarize: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            repo: DEFAULT_REPO.into(),
            git: None,
            reference: None,
            source: None,
            features: Vec::new(),
            no_default_features: false,
            system_ffmpeg: false,
            sign: None,
            notarize: None,
        }
    }
}

/// OpenQ's repository.
pub const DEFAULT_REPO: &str = "iagonunez/OpenQ";

/// OpenQ's version from a source tree's Cargo.toml.
pub fn source_version(src: &Path) -> Result<String> {
    let toml = std::fs::read_to_string(src.join("Cargo.toml")).with_context(|| format!("no Cargo.toml in {}", src.display()))?;
    let mut in_package = false;
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package && let Some(v) = line.strip_prefix("version")
            && let Some(v) = v.trim_start().strip_prefix('=') {
                return Ok(v.trim().trim_matches('"').to_string());
            }
    }
    anyhow::bail!("no package version in {}", src.join("Cargo.toml").display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_cargo_toml() {
        let dir = std::env::temp_dir().join(format!("oqman-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"openq\"\nversion = \"0.3.1\"\n[dependencies]\nversion = \"9\"\n").unwrap();
        assert_eq!(source_version(&dir).unwrap(), "0.3.1");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_state_loads() {
        let s: State = serde_json::from_str(r#"{"commit":"abc","options":{"repo":"a/b"}}"#).unwrap();
        assert_eq!(s.options.repo, "a/b");
        assert!(!s.options.system_ffmpeg);
    }
}
