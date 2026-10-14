//! macOS: Xcode Command Line Tools (Apple's licence, accepted in Apple's own dialog), an
//! LGPL-only FFmpeg compiled from the official source, and `~/Applications/OpenQ.app` with
//! FFmpeg inside, code-signed (ad hoc, or with the user's Developer ID).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::{BuildEnv, Built, Ffmpeg, Wanted};
use crate::net::{Http, Sum};
use crate::state::{Dirs, Options};
use crate::{ui, util};

/// FFmpeg release built for the app, and its tarball's SHA-256 (checked on every download).
const FFMPEG: (&str, &str) = ("9.0.2", "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e");
/// NASM (BSD-2-Clause), needed for FFmpeg's x86 assembly on Intel Macs without one.
const NASM: (&str, &str) = ("3.02", "87336eba53b4acfe917424ab5d500d2b0054d9f5148d35c2273ccf2cfb712f0d");
/// Bump when the FFmpeg configuration below changes, so existing builds are redone.
const FFMPEG_CONFIG_REV: u32 = 1;

/// The FFmpeg configuration: LGPL only (GPL, version-3 and nonfree parts off), no
/// auto-detection (so nothing else installed on the Mac, such as Homebrew's x264, sneaks in),
/// `@rpath` install names so the app loads the copies in its bundle.
fn configure_args(prefix: &Path) -> Vec<String> {
    let mut args = vec![format!("--prefix={}", prefix.display())];
    args.extend(
        [
            "--install-name-dir=@rpath", "--enable-shared", "--disable-static", "--disable-gpl", "--disable-nonfree",
            "--disable-version3", "--disable-programs", "--disable-doc", "--disable-autodetect", "--enable-videotoolbox",
            "--enable-audiotoolbox", "--enable-avfoundation", "--enable-zlib", "--enable-bzlib", "--enable-pic",
        ]
        .map(String::from),
    );
    args
}

fn clt_installed() -> bool {
    util::output(Command::new("xcode-select").arg("-p")).is_some_and(|p| Path::new(&p).exists())
}

/// Xcode Command Line Tools, FFmpeg.
pub fn prepare(dirs: &Dirs, http: &Http, opts: &Options, yes: bool) -> Result<BuildEnv> {
    ui::step("Build dependencies");
    if clt_installed() {
        ui::info("Xcode Command Line Tools: ok");
    } else {
        ui::info("The Xcode Command Line Tools (Apple's compiler and linker) are needed. macOS shows Apple's");
        ui::info("installer and licence; oqman only starts it.");
        if !ui::confirm("Open Apple's installer for the Command Line Tools?", yes)? {
            bail!("The Command Line Tools are needed: run `xcode-select --install`, then oqman again.");
        }
        let _ = util::run(Command::new("xcode-select").arg("--install"));
        bail!("Finish installing the Command Line Tools in Apple's window, then run oqman again.");
    }
    let mut env = BuildEnv { no_default_features: opts.no_default_features, features: opts.features.clone(), ..Default::default() };
    if !Wanted::from(opts.no_default_features, &opts.features).video {
        return Ok(env);
    }
    if opts.system_ffmpeg {
        let ok = util::output(Command::new("pkg-config").args(["--exists", "libavcodec", "libavformat", "libavdevice", "libswscale"])).is_some();
        if !ok {
            bail!("--system-ffmpeg: no FFmpeg found with pkg-config (Homebrew: `brew install ffmpeg pkg-config`).");
        }
        ui::info("FFmpeg: the system's (pkg-config); the app will use it from where it is installed.");
        return Ok(env);
    }
    let ff = ensure_ffmpeg(dirs, http)?;
    env.env.push(("FFMPEG_DIR".into(), ff.dir.clone().into()));
    env.env.push(("PKG_CONFIG_PATH".into(), ff.dir.join("lib/pkgconfig").into()));
    env.ffmpeg = Some(ff);
    Ok(env)
}

fn ffmpeg_dir(dirs: &Dirs) -> PathBuf {
    dirs.deps().join(format!("ffmpeg-{}-lgpl", FFMPEG.0))
}

fn ffmpeg_info(prefix: &Path) -> Ffmpeg {
    let lic = prefix.join("share/oqman");
    Ffmpeg {
        dir: prefix.to_path_buf(),
        licence_files: ["COPYING.LGPLv2.1", "LICENSE.md", "configure-output.txt"].iter().map(|f| lic.join(f)).collect(),
        origin: format!(
            "FFmpeg {} compiled on this Mac by oqman from the official source tarball \
             (https://ffmpeg.org/releases/ffmpeg-{}.tar.xz, SHA-256 {}); configuration in configure-output.txt.",
            FFMPEG.0, FFMPEG.0, FFMPEG.1
        ),
    }
}

/// Builds FFmpeg once (several minutes) into oqman's home; later installs reuse it.
fn ensure_ffmpeg(dirs: &Dirs, http: &Http) -> Result<Ffmpeg> {
    let prefix = ffmpeg_dir(dirs);
    let marker = prefix.join(".oqman-complete");
    if std::fs::read_to_string(&marker).is_ok_and(|s| s.trim() == FFMPEG_CONFIG_REV.to_string()) {
        ui::info(format!("FFmpeg {} (LGPL): {}", FFMPEG.0, prefix.display()));
        return Ok(ffmpeg_info(&prefix));
    }
    ui::step(format!("Compiling FFmpeg {} (LGPL-only; takes a few minutes, once)", FFMPEG.0));
    let mut path = Vec::new();
    if std::env::consts::ARCH == "x86_64" && !util::have("nasm", "-v") {
        path.push(ensure_nasm(dirs, http)?);
    }
    let tarball = dirs.downloads().join(format!("ffmpeg-{}.tar.xz", FFMPEG.0));
    http.download(&format!("https://ffmpeg.org/releases/ffmpeg-{}.tar.xz", FFMPEG.0), &tarball, Sum::Sha256(FFMPEG.1))?;
    let work = dirs.deps().join("ffmpeg-build");
    util::extract(&tarball, &work, 1)?;
    util::remove(&prefix)?;
    let lic = prefix.join("share/oqman");
    std::fs::create_dir_all(&lic)?;

    let path_var = path.iter().fold(std::env::var_os("PATH").unwrap_or_default(), |acc, p| {
        let mut v = vec![p.clone()];
        v.extend(std::env::split_paths(&acc));
        std::env::join_paths(v).unwrap_or_default()
    });
    let log = lic.join("configure-output.txt");
    let mut configure = Command::new("./configure");
    configure.current_dir(&work).args(configure_args(&prefix)).env("PATH", &path_var).stdout(File::create(&log)?);
    util::run(&mut configure)?;
    let out = std::fs::read_to_string(&log)?;
    if !out.lines().any(|l| l.trim() == "License: LGPL version 2.1 or later") {
        bail!("FFmpeg's configuration isn't LGPL-only (see {}); stopping.", log.display());
    }
    util::run(Command::new("make").arg(format!("-j{}", util::jobs())).current_dir(&work).env("PATH", &path_var))?;
    util::run(Command::new("make").arg("install").current_dir(&work).env("PATH", &path_var))?;
    for f in ["COPYING.LGPLv2.1", "LICENSE.md"] {
        util::copy(&work.join(f), &lic.join(f))?;
    }
    util::remove(&work)?;
    std::fs::write(&marker, FFMPEG_CONFIG_REV.to_string())?;
    Ok(ffmpeg_info(&prefix))
}

fn ensure_nasm(dirs: &Dirs, http: &Http) -> Result<PathBuf> {
    let prefix = dirs.deps().join(format!("nasm-{}", NASM.0));
    let bin = prefix.join("bin");
    if bin.join("nasm").is_file() {
        return Ok(bin);
    }
    ui::info(format!("Compiling NASM {} (assembler for FFmpeg on Intel)", NASM.0));
    let tarball = dirs.downloads().join(format!("nasm-{}.tar.xz", NASM.0));
    http.download(
        &format!("https://www.nasm.us/pub/nasm/releasebuilds/{0}/nasm-{0}.tar.xz", NASM.0),
        &tarball,
        Sum::Sha256(NASM.1),
    )?;
    let work = dirs.deps().join("nasm-build");
    util::extract(&tarball, &work, 1)?;
    util::run(Command::new("./configure").arg(format!("--prefix={}", prefix.display())).current_dir(&work))?;
    util::run(Command::new("make").arg(format!("-j{}", util::jobs())).current_dir(&work))?;
    util::run(Command::new("make").arg("install").current_dir(&work))?;
    util::remove(&work)?;
    Ok(bin)
}

fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>OpenQ</string>
  <key>CFBundleDisplayName</key><string>OpenQ</string>
  <key>CFBundleIdentifier</key><string>io.github.iagonunez.openq</string>
  <key>CFBundleExecutable</key><string>OpenQ</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>OpenQ</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Copyright 2026 Iago Núñez. Apache-2.0.</string>
  <key>NSLocalNetworkUsageDescription</key><string>OpenQ uses the local network for remote devices and OSC show control.</string>
  <key>NSMicrophoneUsageDescription</key><string>OpenQ plays live audio inputs in Mic cues and reads LTC timecode.</string>
  <key>NSCameraUsageDescription</key><string>OpenQ shows live cameras on stages in Camera cues.</string>
</dict>
</plist>
"#
    )
}

/// Hardened-runtime entitlements for a Developer ID signature (microphone and camera access).
const ENTITLEMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.device.audio-input</key><true/>
  <key>com.apple.security.device.camera</key><true/>
</dict>
</plist>
"#;

/// Copies the dylibs (files and their version symlinks) from `lib` into `dest`.
fn copy_dylibs(lib: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(lib)?.flatten() {
        let p = entry.path();
        if !p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".dylib")) {
            continue;
        }
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(&p)?, &to)?;
        } else {
            std::fs::copy(&p, &to)?;
        }
    }
    Ok(())
}

fn applications() -> Result<PathBuf> {
    Ok(util::home_dir()?.join("Applications"))
}

/// Builds `~/Applications/OpenQ.app` and signs it.
pub fn install(_dirs: &Dirs, opts: &Options, built: &Built, _yes: bool) -> Result<Vec<PathBuf>> {
    ui::step("Installing");
    let apps = applications()?;
    let app = apps.join("OpenQ.app");
    let staging = apps.join(".OpenQ.app.oqman");
    util::remove(&staging)?;
    let contents = staging.join("Contents");
    let exe = contents.join("MacOS/OpenQ");
    util::copy(&built.exe, &exe)?;
    std::fs::write(contents.join("Info.plist"), info_plist(built.version))?;
    // The app icon (OpenQ 0.2 and later ship it; Info.plist names it either way).
    let icns = built.src.join("assets/icon/OpenQ.icns");
    if icns.is_file() {
        util::copy(&icns, &contents.join("Resources/OpenQ.icns"))?;
    }
    super::write_licences(built, &contents.join("Resources"))?;
    if let Some(ff) = &built.env.ffmpeg {
        copy_dylibs(&ff.dir.join("lib"), &contents.join("Frameworks"))?;
        util::run(Command::new("install_name_tool").args(["-add_rpath", "@executable_path/../Frameworks"]).arg(&exe))?;
        // Only the bundle's FFmpeg may be loaded.
        let deps = util::output(Command::new("otool").arg("-L").arg(&exe)).unwrap_or_default();
        if deps.lines().any(|l| (l.contains("libav") || l.contains("libsw")) && !l.trim_start().starts_with("@rpath")) {
            bail!("the program links FFmpeg by absolute path:\n{deps}");
        }
    }
    sign(&staging, opts)?;
    util::remove(&app)?;
    std::fs::rename(&staging, &app).with_context(|| format!("couldn't move the app to {}", app.display()))?;
    if let Some(profile) = &opts.notarize {
        notarize(&app, profile)?;
    }
    ui::info(format!("OpenQ {} installed: {}", built.version, app.display()));
    Ok(vec![app])
}

/// Signs the bundle: with `opts.sign` (a keychain identity, e.g. "Developer ID Application:
/// Name (TEAMID)") using the hardened runtime and a timestamp, otherwise ad hoc. An app built on
/// this Mac carries no quarantine flag, so Gatekeeper opens it either way; Apple silicon only
/// requires a valid signature, which the ad hoc one is.
fn sign(app: &Path, opts: &Options) -> Result<()> {
    let mut cmd = Command::new("codesign");
    cmd.args(["--force", "--deep"]);
    match &opts.sign {
        Some(identity) => {
            ui::step(format!("Signing with \"{identity}\""));
            let ent = app.with_extension("entitlements");
            std::fs::write(&ent, ENTITLEMENTS)?;
            cmd.args(["--options", "runtime", "--timestamp", "--entitlements"]).arg(&ent).args(["--sign", identity]).arg(app);
            let r = util::run(&mut cmd);
            let _ = std::fs::remove_file(&ent);
            r?;
        }
        None => {
            ui::info("Signing ad hoc (use --sign \"Developer ID Application: ...\" for a Developer ID signature)");
            util::run(cmd.args(["--sign", "-"]).arg(app))?;
        }
    }
    util::run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(app))
}

/// Sends the signed app to Apple's notary service and staples the ticket, so the app also opens
/// without warnings when copied to other Macs. `profile` is a `xcrun notarytool
/// store-credentials` keychain profile.
fn notarize(app: &Path, profile: &str) -> Result<()> {
    ui::step("Notarising (Apple's notary service; usually a few minutes)");
    let zip = app.with_extension("zip");
    util::run(Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(app).arg(&zip))?;
    let r = util::run(Command::new("xcrun").args(["notarytool", "submit"]).arg(&zip).args(["--keychain-profile", profile, "--wait"]));
    let _ = std::fs::remove_file(&zip);
    r?;
    util::run(Command::new("xcrun").args(["stapler", "staple"]).arg(app))
}

/// Nothing outside the recorded files on macOS.
pub fn uninstall_extra() -> Result<()> {
    Ok(())
}

/// Prints the state of each dependency.
pub fn doctor(dirs: &Dirs, opts: &Options) {
    ui::info(format!("Xcode Command Line Tools: {}", if clt_installed() { "ok" } else { "missing" }));
    if opts.system_ffmpeg {
        ui::info("FFmpeg: the system's (pkg-config)");
    } else {
        let built = ffmpeg_dir(dirs).join(".oqman-complete").is_file();
        ui::info(format!("FFmpeg {} (LGPL): {}", FFMPEG.0, if built { "built" } else { "not built yet" }));
    }
    let identities = util::output(Command::new("security").args(["find-identity", "-v", "-p", "codesigning"])).unwrap_or_default();
    ui::info(format!("Signing identities: {}", identities.lines().last().unwrap_or("none")));
}
