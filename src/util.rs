//! Running programs, archives and file helpers.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::ui;

/// The command line as it would be typed, for messages.
pub fn show(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().into_owned();
    for a in cmd.get_args() {
        let a = a.to_string_lossy();
        if a.is_empty() || a.contains(' ') {
            s += &format!(" \"{a}\"");
        } else {
            s += &format!(" {a}");
        }
    }
    s
}

/// Runs a command with the terminal attached and fails unless it succeeds.
pub fn run(cmd: &mut Command) -> Result<()> {
    ui::info(format!("$ {}", show(cmd)));
    let status = cmd.status().with_context(|| format!("couldn't start {}", cmd.get_program().to_string_lossy()))?;
    if !status.success() {
        bail!("`{}` failed ({status})", show(cmd));
    }
    Ok(())
}

#[cfg(windows)]
/// Like [`run`], but returns the exit code instead of failing on one (installers that use
/// codes such as 3010 "restart needed").
pub fn run_code(cmd: &mut Command) -> Result<i32> {
    ui::info(format!("$ {}", show(cmd)));
    let status = cmd.status().with_context(|| format!("couldn't start {}", cmd.get_program().to_string_lossy()))?;
    Ok(status.code().unwrap_or(-1))
}

/// Runs a command quietly and returns its trimmed standard output if it succeeded.
pub fn output(cmd: &mut Command) -> Option<String> {
    let out = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether `program` runs (looked up on PATH).
pub fn have(program: impl AsRef<OsStr>, version_arg: &str) -> bool {
    output(Command::new(program).arg(version_arg)).is_some()
}

/// The system `tar` (bsdtar on Windows and macOS, which also reads zip files). On Windows the
/// one in System32 is used, so a GNU tar from Git or MSYS on PATH doesn't take its place.
fn tar() -> Command {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        Command::new(Path::new(&root).join("System32").join("tar.exe"))
    } else {
        Command::new("tar")
    }
}

/// Extracts an archive (.tar.gz, .tar.xz, or .zip on Windows and macOS) into a new, empty `dest`,
/// dropping `strip` leading path components.
pub fn extract(archive: &Path, dest: &Path, strip: u32) -> Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest).with_context(|| format!("couldn't clear {}", dest.display()))?;
    }
    std::fs::create_dir_all(dest)?;
    let mut cmd = tar();
    cmd.arg("-xf").arg(archive).arg("-C").arg(dest);
    if strip > 0 {
        cmd.arg(format!("--strip-components={strip}"));
    }
    let out = cmd.output().context("couldn't run tar")?;
    if !out.status.success() {
        bail!("couldn't extract {}: {}", archive.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Copies a file, creating the destination's folder.
pub fn copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(from, to).with_context(|| format!("couldn't copy {} to {}", from.display(), to.display()))?;
    Ok(())
}

#[cfg(windows)]
/// The first file named `name` under `dir` (depth first, sorted, so the result is stable).
pub fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in &entries {
        if p.is_file() && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)) {
            return Some(p.clone());
        }
    }
    entries.iter().filter(|p| p.is_dir()).find_map(|p| find_file(p, name))
}

/// Removes a file or folder if it exists.
pub fn remove(path: &Path) -> Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
    .with_context(|| format!("couldn't remove {}", path.display()))
}

/// The user's home folder.
pub fn home_dir() -> Result<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).with_context(|| format!("{var} isn't set"))
}

#[cfg(target_os = "macos")]
/// The number of CPUs, for parallel `make`.
pub fn jobs() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}
