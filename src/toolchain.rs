//! The Rust toolchain: the system's if it has a recent enough one, otherwise a private copy
//! installed with the official `rustup-init` into openqup's home (the user's PATH and shell
//! files are left alone).

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::net::{self, Http, Sum};
use crate::state::Dirs;
use crate::{ui, util};

/// Oldest Rust that builds OpenQ (edition 2024).
const MIN_RUST: (u64, u64) = (1, 85);

/// A usable toolchain.
pub struct Rust {
    /// The `cargo` to run.
    pub cargo: PathBuf,
    /// Environment for it (`RUSTUP_HOME`/`CARGO_HOME` for the private toolchain).
    pub env: Vec<(String, OsString)>,
    /// Folder to put first on PATH (so build scripts find the same `rustc`).
    pub bin: Option<PathBuf>,
    /// Installed by openqup.
    pub private: bool,
}

impl Rust {
    /// A command for one of the toolchain's programs (`cargo`, `rustc`, `rustup`).
    pub fn command(&self, program: &str) -> Command {
        let exe = match &self.bin {
            Some(bin) => bin.join(format!("{program}{}", std::env::consts::EXE_SUFFIX)),
            None => PathBuf::from(program),
        };
        let mut cmd = Command::new(exe);
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        if let Some(bin) = &self.bin {
            cmd.env("PATH", prepend_path(bin));
        }
        cmd
    }

    /// `rustc --version`, parsed.
    fn version(&self) -> Option<(u64, u64)> {
        parse_rustc_version(&util::output(self.command("rustc").arg("--version"))?)
    }
}

/// PATH with `dir` first.
pub fn prepend_path(dir: &std::path::Path) -> OsString {
    let mut paths = vec![dir.to_path_buf()];
    if let Some(p) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&p));
    }
    std::env::join_paths(paths).unwrap_or_default()
}

fn private(dirs: &Dirs) -> Rust {
    let root = dirs.rust();
    let bin = root.join("cargo").join("bin");
    Rust {
        cargo: bin.join(format!("cargo{}", std::env::consts::EXE_SUFFIX)),
        env: vec![("RUSTUP_HOME".into(), root.join("rustup").into()), ("CARGO_HOME".into(), root.join("cargo").into())],
        bin: Some(bin),
        private: true,
    }
}

/// The toolchain to build with, or `None` when there's none.
pub fn find(dirs: &Dirs) -> Option<Rust> {
    let own = private(dirs);
    if own.cargo.is_file() {
        return Some(own);
    }
    if util::have("cargo", "--version") && util::have("rustc", "--version") {
        return Some(Rust { cargo: "cargo".into(), env: Vec::new(), bin: None, private: false });
    }
    // Installed by rustup but not on this shell's PATH yet.
    let bin = util::home_dir().ok()?.join(".cargo").join("bin");
    let rust = Rust { cargo: bin.join(format!("cargo{}", std::env::consts::EXE_SUFFIX)), env: Vec::new(), bin: Some(bin), private: false };
    rust.cargo.is_file().then_some(rust)
}

/// Finds a recent enough toolchain or installs one (after asking).
pub fn ensure(dirs: &Dirs, http: &Http, yes: bool) -> Result<Rust> {
    ui::step("Rust toolchain");
    if let Some(rust) = find(dirs) {
        match rust.version() {
            Some(v) if v >= MIN_RUST => {
                ui::info(format!("{} Rust {}.{} ({})", if rust.private { "openqup's" } else { "System" }, v.0, v.1, rust.cargo.display()));
                return Ok(rust);
            }
            v => {
                let found = v.map(|v| format!("{}.{}", v.0, v.1)).unwrap_or_else(|| "unknown".into());
                ui::warn(format!("Rust {found} is older than {}.{}, which OpenQ needs", MIN_RUST.0, MIN_RUST.1));
                if util::output(rust.command("rustup").arg("--version")).is_some()
                    && ui::confirm("Update it with `rustup update stable`?", yes)?
                {
                    util::run(rust.command("rustup").args(["update", "stable", "--no-self-update"]))?;
                    return Ok(rust);
                }
                if !rust.private {
                    bail!("Update Rust (https://rustup.rs) and run openqup again.");
                }
            }
        }
    }
    install(dirs, http, yes)
}

/// Updates openqup's private toolchain (the system's is the user's to update).
pub fn update(rust: &Rust) -> Result<()> {
    if rust.private {
        ui::step("Updating openqup's Rust toolchain");
        util::run(rust.command("rustup").args(["update", "stable", "--no-self-update"]))?;
    }
    Ok(())
}

fn install(dirs: &Dirs, http: &Http, yes: bool) -> Result<Rust> {
    let triple = host_triple()?;
    ui::info("No Rust toolchain found.");
    ui::info(format!(
        "openqup will install Rust (stable, minimal profile) with the official rustup-init into {}.\n    \
         Rust is free software (MIT / Apache-2.0). Your PATH and shell files are not changed.",
        dirs.rust().display()
    ));
    if !ui::confirm("Install Rust?", yes)? {
        bail!("Rust is needed to build OpenQ. Install it from https://rustup.rs and run openqup again.");
    }
    let exe = format!("rustup-init{}", std::env::consts::EXE_SUFFIX);
    let url = format!("https://static.rust-lang.org/rustup/dist/{triple}/{exe}");
    let sum = net::parse_sha256_file(&http.text(&format!("{url}.sha256"))?).context("bad rustup-init checksum file")?;
    let init = dirs.downloads().join(&exe);
    http.download(&url, &init, Sum::Sha256(&sum))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&init, std::fs::Permissions::from_mode(0o755))?;
    }
    let rust = private(dirs);
    let mut cmd = Command::new(&init);
    cmd.args(["-y", "--no-modify-path", "--profile", "minimal", "--default-toolchain", "stable", "--default-host", triple]);
    cmd.envs(rust.env.iter().map(|(k, v)| (k, v)));
    util::run(&mut cmd)?;
    ui::info("(openqup runs this toolchain itself: nothing needs adding to PATH.)");
    let _ = std::fs::remove_file(&init);
    if !rust.cargo.is_file() {
        bail!("rustup-init finished but {} is missing", rust.cargo.display());
    }
    Ok(rust)
}

/// The Rust host triple for this computer.
pub fn host_triple() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        (os, arch) => bail!("openqup doesn't support {os} on {arch} yet"),
    })
}

fn parse_rustc_version(text: &str) -> Option<(u64, u64)> {
    // "rustc 1.94.1 (29ea6fb6a 2026-03-24)"
    let mut parts = text.split_whitespace().nth(1)?.split(['.', '-']);
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustc_versions() {
        assert_eq!(parse_rustc_version("rustc 1.94.1 (29ea6fb6a 2026-03-24)"), Some((1, 94)));
        assert_eq!(parse_rustc_version("rustc 1.96.0-nightly (abc 2026-05-01)"), Some((1, 96)));
        assert!(parse_rustc_version("rustc 1.84.0").unwrap() < MIN_RUST);
    }
}
