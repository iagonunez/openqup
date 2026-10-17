//! oqman installs OpenQ by building it from source on this computer, the way rustup installs
//! Rust: it gets a Rust toolchain, the build dependencies (FFmpeg and friends, each from its
//! official source and under its own licence), the source of the chosen OpenQ version, builds
//! it and installs the app with a menu entry. `oqman update` repeats that for newer versions.

mod net;
mod platform;
mod python;
mod source;
mod state;
mod toolchain;
mod ui;
mod util;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::net::Http;
use crate::platform::{BuildEnv, Built};
use crate::state::{Dirs, Options, State};

const USAGE: &str = "\
oqman — installs OpenQ by building it from source

Usage:
  oqman install [options]   Build and install OpenQ (the newest release, unless --ref)
  oqman update [-y] [--force]  Rebuild if the followed version or oqman changed (same options as install)
  oqman uninstall [--purge] [-y]  Remove the app (--purge: also toolchain, FFmpeg, caches)
  oqman versions            List the versions that can be installed
  oqman doctor              Show what is installed and what the build needs
  oqman --version | --help

Install options:
  --ref <tag|branch|commit>   Version to build (default: newest vX.Y.Z tag, else the main branch)
  --repo <owner/name>         GitHub repository (default: iagonunez/OpenQ)
  --git <url>                 Get the source with git instead (e.g. git@github.com:owner/OpenQ.git;
                              uses your SSH keys / git credentials)
  --source <dir>              Build a local source folder instead of downloading
  --features <a,b>            Extra cargo features
  --no-default-features       No default features (no video, no PipeWire), plus --features
  --no-lighting               Cues only: leave out the lighting workspace (Lighting view, Light
                              cues, fixture downloads); --lighting brings it back
  --system-ffmpeg             macOS: link the system's (Homebrew) FFmpeg instead of compiling one
  --sign <identity>           Code-sign the app. macOS: keychain identity, e.g.
                              \"Developer ID Application: Name (TEAMID)\" (default: ad hoc).
                              Windows: certificate SHA-1 thumbprint (default: unsigned)
  --notarize <profile>        macOS: notarise with this `notarytool store-credentials` profile
  -y, --yes                   Don't ask; accept the installs oqman proposes

Environment:
  OPENQ_GITHUB_TOKEN          GitHub token (or GH_TOKEN / GITHUB_TOKEN / `gh auth login`);
                              needed while the OpenQ repository is private
  OQMAN_HOME                oqman's folder (sources, build cache, dependencies)
";

struct Args {
    command: String,
    yes: bool,
    force: bool,
    purge: bool,
    opts: Options,
    /// Options given on this command line (update keeps the saved ones otherwise).
    given: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args> {
    let mut a = Args { command: String::new(), yes: false, force: false, purge: false, opts: Options::default(), given: false };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" | "help" => a.command = "help".into(),
            "-V" | "--version" => a.command = "version".into(),
            "-y" | "--yes" => a.yes = true,
            "--force" => a.force = true,
            "--purge" => a.purge = true,
            "--ref" => (a.opts.reference, a.given) = (Some(value("--ref")?), true),
            "--repo" => (a.opts.repo, a.given) = (value("--repo")?, true),
            "--git" => (a.opts.git, a.given) = (Some(value("--git")?), true),
            "--source" => {
                let dir = PathBuf::from(value("--source")?);
                a.opts.source = Some(std::fs::canonicalize(&dir).with_context(|| format!("no folder {}", dir.display()))?);
                a.given = true;
            }
            "--features" => {
                a.opts.features = value("--features")?.split([',', ' ']).filter(|f| !f.is_empty()).map(String::from).collect();
                a.given = true;
            }
            "--no-default-features" => (a.opts.no_default_features, a.given) = (true, true),
            "--no-lighting" => (a.opts.no_lighting, a.given) = (true, true),
            "--lighting" => (a.opts.no_lighting, a.given) = (false, true),
            "--system-ffmpeg" => (a.opts.system_ffmpeg, a.given) = (true, true),
            "--sign" => (a.opts.sign, a.given) = (Some(value("--sign")?), true),
            "--notarize" => (a.opts.notarize, a.given) = (Some(value("--notarize")?), true),
            cmd if !cmd.starts_with('-') && a.command.is_empty() => a.command = cmd.to_string(),
            other => bail!("unknown argument `{other}` (see oqman --help)"),
        }
    }
    if a.opts.notarize.is_some() && a.opts.sign.is_none() {
        bail!("--notarize needs --sign with a Developer ID identity");
    }
    if a.command.is_empty() {
        a.command = "help".into();
    }
    Ok(a)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("\nerror: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = parse(std::env::args().skip(1))?;
    let dirs = Dirs { home: state::home()? };
    match args.command.as_str() {
        "help" => print!("{USAGE}"),
        "version" => println!("oqman {}", env!("CARGO_PKG_VERSION")),
        "install" => {
            let _lock = lock(&dirs)?;
            install(&dirs, &args.opts, args.yes, None)?;
        }
        "update" => {
            let _lock = lock(&dirs)?;
            let saved = dirs.load()?.context("OpenQ isn't installed yet: run `oqman install`")?;
            let opts = if args.given { args.opts.clone() } else { saved.options.clone() };
            install(&dirs, &opts, args.yes, Some((&saved, args.force)))?;
        }
        "uninstall" => {
            let _lock = lock(&dirs)?;
            uninstall(&dirs, args.purge, args.yes)?;
        }
        "versions" => {
            let http = Http::new();
            let opts = match dirs.load()? {
                Some(s) if !args.given => s.options,
                _ => args.opts,
            };
            let mut tags = source::tags(&http, &opts)?;
            tags.retain(|t| source::newest_version([t.as_str()].into_iter()).is_some());
            if tags.is_empty() {
                println!("No releases tagged yet; `oqman install` builds the main branch.");
            }
            for t in tags {
                println!("{t}");
            }
        }
        "doctor" => doctor(&dirs, args.opts)?,
        other => bail!("unknown command `{other}` (see oqman --help)"),
    }
    Ok(())
}

/// Only one oqman at a time works on the same home.
fn lock(dirs: &Dirs) -> Result<std::fs::File> {
    std::fs::create_dir_all(&dirs.home)?;
    let f = std::fs::File::create(dirs.home.join(".lock"))?;
    if f.try_lock().is_err() {
        bail!("another oqman is running");
    }
    Ok(f)
}

fn install(dirs: &Dirs, opts: &Options, yes: bool, update: Option<(&State, bool)>) -> Result<()> {
    let http = Http::new();
    ui::step("OpenQ source");
    let (reference, commit) = match &opts.source {
        Some(dir) => (dir.display().to_string(), source::local_commit(dir)),
        None => {
            let r = source::pick_ref(&http, opts)?;
            let c = source::resolve(&http, opts, &r)?;
            (r, c)
        }
    };
    ui::info(format!("{} at {}", reference, &commit[..commit.len().min(12)]));
    if let Some((saved, force)) = update
        && !force
        && opts.source.is_none()
        && saved.commit == commit
        && saved.options == *opts
        && saved.installer == env!("CARGO_PKG_VERSION")
    {
        ui::info(format!("OpenQ {} is up to date.", saved.version));
        return Ok(());
    }

    let rust = toolchain::ensure(dirs, &http, yes)?;
    if update.is_some() {
        toolchain::update(&rust)?;
    }
    let mut env = platform::prepare(dirs, &http, opts, yes)?;
    let src = match &opts.source {
        Some(dir) => source::Source { dir: dir.clone(), commit: commit.clone(), label: reference.clone() },
        None => source::fetch(&http, dirs, opts, &reference, &commit)?,
    };
    let version = state::source_version(&src.dir)?;
    if opts.no_lighting {
        env.without.push("lighting".into());
        env.features.retain(|f| f != "lighting");
    }
    env.resolve(&src.dir);
    let exe = build(dirs, &rust, &env, &src)?;
    let python = if python::wanted(&src.dir, env.no_default_features, &env.features) {
        Some(python::prepare(dirs, &http)?)
    } else {
        // OpenQ after 0.3 makes effects from formulas: an interpreter kept for 0.3 isn't needed.
        python::forget(dirs)?;
        None
    };
    let built = Built { exe, src: &src.dir, version: &version, env: &env, python: python.as_deref() };
    let mut installed = platform::install(dirs, opts, &built, yes)?;

    // Anything an earlier install created that this one didn't recreate.
    if let Some(old) = dirs.load()? {
        for p in old.installed.iter().filter(|p| !installed.contains(p)) {
            let _ = util::remove(p);
        }
    }
    installed.dedup();
    dirs.save(&State { options: opts.clone(), commit: src.commit.clone(), version: version.clone(), installed_at: platform::now(), installer: env!("CARGO_PKG_VERSION").into(), installed })?;
    ui::step(format!("Done: OpenQ {version} ({})", src.label));
    Ok(())
}

fn build(dirs: &Dirs, rust: &toolchain::Rust, env: &BuildEnv, src: &source::Source) -> Result<PathBuf> {
    ui::step("Building OpenQ (the first build takes several minutes)");
    let mut cmd = rust.command("cargo");
    cmd.args(["build", "--release", "--locked"]).current_dir(&src.dir).env("CARGO_TARGET_DIR", dirs.target());
    if env.no_default_features {
        cmd.arg("--no-default-features");
    }
    if !env.features.is_empty() {
        cmd.arg("--features").arg(env.features.join(","));
    }
    cmd.envs(env.env.iter().map(|(k, v)| (k, v)));
    if !env.path.is_empty() || rust.bin.is_some() {
        let mut paths: Vec<PathBuf> = env.path.clone();
        paths.extend(rust.bin.clone());
        if let Some(p) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&p));
        }
        cmd.env("PATH", std::env::join_paths(paths)?);
    }
    util::run(&mut cmd)?;
    let exe = dirs.target().join("release").join(format!("openq{}", std::env::consts::EXE_SUFFIX));
    if !exe.is_file() {
        bail!("the build finished but {} is missing", exe.display());
    }
    Ok(exe)
}

fn uninstall(dirs: &Dirs, purge: bool, yes: bool) -> Result<()> {
    let state = dirs.load()?;
    match &state {
        Some(s) => {
            ui::step(format!("Uninstalling OpenQ {}", s.version));
            for p in &s.installed {
                ui::info(format!("remove {}", p.display()));
            }
        }
        None if !purge => bail!("OpenQ isn't installed by oqman (nothing to remove)."),
        None => {}
    }
    if purge {
        ui::info(format!("remove {} (toolchain, dependencies, sources, build cache)", dirs.home.display()));
    }
    if !ui::confirm("Remove?", yes)? {
        return Ok(());
    }
    if let Some(s) = &state {
        for p in &s.installed {
            util::remove(p)?;
        }
    }
    platform::uninstall_extra()?;
    dirs.clear()?;
    if purge {
        // The lock file and, on Windows, oqman's own running copy can't go while it runs.
        for entry in std::fs::read_dir(&dirs.home)?.flatten() {
            let name = entry.file_name();
            if name == ".lock" || (cfg!(windows) && name == "bin") {
                continue;
            }
            util::remove(&entry.path())?;
        }
    }
    ui::step("OpenQ removed.");
    Ok(())
}

fn doctor(dirs: &Dirs, given: Options) -> Result<()> {
    let state = dirs.load()?;
    let opts = state.as_ref().map(|s| s.options.clone()).unwrap_or(given);
    ui::step("oqman");
    ui::info(format!("Version {} · home {}", env!("CARGO_PKG_VERSION"), dirs.home.display()));
    ui::info(format!(
        "GitHub token: {}",
        if Http::new().has_token() { "found" } else { "none (needed while the OpenQ repository is private)" }
    ));
    ui::step("OpenQ");
    match &state {
        Some(s) => {
            let from = match (&s.options.source, &s.options.git) {
                (Some(dir), _) => dir.display().to_string(),
                (None, Some(url)) => url.clone(),
                (None, None) => s.options.repo.clone(),
            };
            ui::info(format!("{} at {} from {from}", s.version, &s.commit[..s.commit.len().min(12)]));
            for p in &s.installed {
                ui::info(format!("{} {}", if p.exists() { "ok     " } else { "missing" }, p.display()));
            }
        }
        None => ui::info("not installed"),
    }
    ui::step("Rust");
    match toolchain::find(dirs) {
        Some(rust) => {
            let v = util::output(rust.command("rustc").arg("--version")).unwrap_or_else(|| "rustc doesn't run".into());
            ui::info(format!("{v} ({})", if rust.private { "oqman's" } else { "system" }));
        }
        None => ui::info("not installed (oqman installs it)"),
    }
    ui::step("Build dependencies");
    platform::doctor(dirs, &opts);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Result<Args> {
        parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn arguments() {
        let a = args("install --ref v0.2.0 --features video,fault-injection --no-default-features -y").unwrap();
        assert_eq!(a.command, "install");
        assert!(a.yes && a.given && a.opts.no_default_features);
        assert_eq!(a.opts.reference.as_deref(), Some("v0.2.0"));
        assert_eq!(a.opts.features, ["video", "fault-injection"]);
        assert!(!a.opts.no_lighting);
        assert!(args("install --no-lighting").unwrap().opts.no_lighting);
        assert!(!args("update --no-lighting --lighting").unwrap().opts.no_lighting);
        let u = args("update -y").unwrap();
        assert!(!u.given);
        assert!(args("install --notarize prof").is_err());
        assert!(args("install --bogus").is_err());
        assert_eq!(args("").unwrap().command, "help");
    }
}
