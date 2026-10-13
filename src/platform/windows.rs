//! Windows: Microsoft's C++ Build Tools (installed by Microsoft's own installer, where the user
//! accepts Microsoft's licence), LLVM's libclang (official release, Apache-2.0 with LLVM
//! exception), FFmpeg's LGPL-only shared build from BtbN, and the app in
//! `%LOCALAPPDATA%\Programs\OpenQ` with a Start menu shortcut and an "Installed apps" entry.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::{BuildEnv, Built, Ffmpeg, Wanted};
use crate::net::{Asset, Http, Sum};
use crate::state::{Dirs, Options};
use crate::{ui, util};

/// FFmpeg release series of the bundled libraries (BtbN builds each series' latest point release).
const FFMPEG_SERIES: &str = "9.0";
/// LLVM's Windows installer, used only for libclang (bindgen reads FFmpeg's headers with it).
const LLVM: (&str, &str) = ("23.1.3", "5153f848ac87553118340f16f8d3cd4351d729bcba6e4785c6a4a9af29676de9");
/// Microsoft's bootstrapper for the Visual Studio 2022 Build Tools.
const VS_BUILD_TOOLS: &str = "https://aka.ms/vs/17/release/vs_BuildTools.exe";
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenQ";
const TIMESTAMP_URL: &str = "http://timestamp.digicert.com";

fn program_files_x86() -> PathBuf {
    std::env::var_os("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|| r"C:\Program Files (x86)".into())
}

/// The newest Visual Studio installation with the x64 C++ tools.
fn msvc() -> Option<String> {
    let vswhere = program_files_x86().join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    util::output(Command::new(vswhere).args([
        "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property",
        "installationPath",
    ]))
    .filter(|p| !p.is_empty())
}

fn libclang(dirs: &Dirs) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("LIBCLANG_PATH") {
        candidates.push(PathBuf::from(p).join("libclang.dll"));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join(r"LLVM\bin\libclang.dll"));
    }
    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    util::find_file(&dirs.deps().join(format!("llvm-{}", LLVM.0)), "libclang.dll")
}

fn ensure_msvc(http: &Http, dirs: &Dirs, yes: bool) -> Result<()> {
    if let Some(path) = msvc() {
        ui::info(format!("C++ Build Tools: {path}"));
        return Ok(());
    }
    ui::info("Rust needs Microsoft's C++ Build Tools (the MSVC linker and the Windows SDK).");
    ui::info("oqman will download Microsoft's installer and open it with the C++ tools selected.");
    ui::info("Microsoft's licence applies; you accept it in Microsoft's installer:");
    ui::info("  https://visualstudio.microsoft.com/license-terms/vs2022-ga-diagnosticbuildtools/");
    ui::info("(If you already have Visual Studio, add the \"Desktop development with C++\" workload instead.)");
    if !ui::confirm("Download and open the Visual Studio Build Tools installer?", yes)? {
        bail!("The C++ Build Tools are needed to build OpenQ.");
    }
    let exe = dirs.downloads().join("vs_BuildTools.exe");
    http.download(VS_BUILD_TOOLS, &exe, Sum::None)?;
    check_microsoft_signature(&exe)?;
    let code = util::run_code(Command::new(&exe).args([
        "--add", "Microsoft.VisualStudio.Workload.VCTools", "--includeRecommended", "--wait", "--norestart",
    ]))?;
    let _ = std::fs::remove_file(&exe);
    match code {
        0 => {}
        3010 => ui::warn("The Build Tools ask for a restart; restart Windows if the build fails."),
        c => bail!("The Build Tools installer ended with code {c}."),
    }
    if msvc().is_none() {
        bail!("The C++ Build Tools still aren't found; install the \"Desktop development with C++\" workload.");
    }
    Ok(())
}

/// The Visual Studio bootstrapper changes with every update, so it has no fixed checksum: check
/// that it carries a valid Microsoft Authenticode signature instead.
fn check_microsoft_signature(exe: &Path) -> Result<()> {
    let script = "$s = Get-AuthenticodeSignature -LiteralPath $env:OQMAN_FILE; \
                  if ($s.Status -eq 'Valid' -and $s.SignerCertificate.Subject -match 'O=Microsoft Corporation') { 'ok' } else { $s.Status }";
    let out = util::output(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]).env("OQMAN_FILE", exe));
    if out.as_deref() != Some("ok") {
        let _ = std::fs::remove_file(exe);
        bail!("{} isn't signed by Microsoft ({}); deleted it.", exe.display(), out.unwrap_or_default());
    }
    ui::info("Signature OK (Microsoft Corporation)");
    Ok(())
}

fn ensure_libclang(http: &Http, dirs: &Dirs) -> Result<PathBuf> {
    if let Some(dll) = libclang(dirs) {
        ui::info(format!("libclang: {}", dll.display()));
        return Ok(dll);
    }
    ui::info(format!("Getting libclang from the official LLVM {} release (Apache-2.0 with LLVM exception)", LLVM.0));
    let msi = dirs.downloads().join(format!("LLVM-{}-win64.msi", LLVM.0));
    let url = format!("https://github.com/llvm/llvm-project/releases/download/llvmorg-{0}/LLVM-{0}-win64.msi", LLVM.0);
    http.download(&url, &msi, Sum::Sha256(LLVM.1))?;
    // An administrative install only unpacks the files: nothing is registered or added to PATH.
    let dir = dirs.deps().join(format!("llvm-{}", LLVM.0));
    util::remove(&dir)?;
    std::fs::create_dir_all(&dir)?;
    let code = util::run_code(Command::new("msiexec").arg("/a").arg(&msi).arg("/qn").arg(format!("TARGETDIR={}", dir.display())))?;
    if code != 0 {
        bail!("unpacking LLVM failed (msiexec code {code})");
    }
    let _ = std::fs::remove_file(&msi);
    libclang(dirs).context("libclang.dll not found in the LLVM package")
}

/// A BtbN build of this series: a dated "autobuild" (its name carries the exact FFmpeg commit),
/// falling back to the rolling "latest" one.
fn btbn_asset(http: &Http) -> Result<Asset> {
    let suffix = format!("-win64-lgpl-shared-{FFMPEG_SERIES}.zip");
    let prefix = format!("ffmpeg-n{FFMPEG_SERIES}");
    let assets = http.release_assets("BtbN/FFmpeg-Builds", 10)?;
    let matches = |name: &str| {
        name.ends_with(&suffix) && name.strip_prefix(&prefix).is_some_and(|rest| rest.starts_with('.') || rest.starts_with('-'))
    };
    assets
        .iter()
        .find(|a| a.tag.starts_with("autobuild-") && matches(&a.name))
        .or_else(|| assets.iter().find(|a| matches(&a.name)))
        .cloned()
        .with_context(|| format!("no BtbN lgpl-shared build of FFmpeg {FFMPEG_SERIES} found"))
}

fn ffmpeg_info(dir: &Path) -> Ffmpeg {
    let build = std::fs::read_to_string(dir.join("BUILD.txt")).unwrap_or_default();
    Ffmpeg {
        dir: dir.to_path_buf(),
        licence_files: vec![dir.join("LICENSE.txt"), dir.join("BUILD.txt")],
        origin: format!("BtbN's LGPL-only shared build (https://github.com/BtbN/FFmpeg-Builds), {}", build.lines().next().unwrap_or("")),
    }
}

/// The installed BtbN build of the series, if any (the newest one).
fn installed_ffmpeg(dirs: &Dirs) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dirs.deps())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy();
            n.starts_with(&format!("ffmpeg-n{FFMPEG_SERIES}")) && n.contains("lgpl-shared") && p.join(".oqman-complete").is_file()
        })
        .collect();
    found.sort();
    found.pop()
}

fn ensure_ffmpeg(http: &Http, dirs: &Dirs) -> Result<Ffmpeg> {
    if let Some(dir) = installed_ffmpeg(dirs) {
        ui::info(format!("FFmpeg (LGPL): {}", dir.display()));
        return Ok(ffmpeg_info(&dir));
    }
    let Asset { tag, name, url, sha256: digest } = btbn_asset(http)?;
    if !name.contains("lgpl") {
        bail!("{name} isn't an LGPL build");
    }
    ui::info(format!("FFmpeg {FFMPEG_SERIES} LGPL shared build from BtbN ({tag})"));
    let zip = dirs.downloads().join(&name);
    let sum = match &digest {
        Some(d) => Sum::Sha256(d),
        None => Sum::None,
    };
    http.download(&url, &zip, sum)?;
    let dir = dirs.deps().join(name.trim_end_matches(".zip"));
    util::extract(&zip, &dir, 1)?;
    if !dir.join("bin").is_dir() || !dir.join("include/libavcodec").is_dir() {
        bail!("unexpected layout in {name}");
    }
    std::fs::write(dir.join("BUILD.txt"), format!("{name} (BtbN release {tag})\n"))?;
    std::fs::write(dir.join(".oqman-complete"), "")?;
    let _ = std::fs::remove_file(&zip);
    Ok(ffmpeg_info(&dir))
}

/// C++ Build Tools, libclang and FFmpeg.
pub fn prepare(dirs: &Dirs, http: &Http, opts: &Options, yes: bool) -> Result<BuildEnv> {
    ui::step("Build dependencies");
    ensure_msvc(http, dirs, yes)?;
    let mut env = BuildEnv { no_default_features: opts.no_default_features, features: opts.features.clone(), ..Default::default() };
    if Wanted::from(opts.no_default_features, &opts.features).video {
        let clang = ensure_libclang(http, dirs)?;
        env.env.push(("LIBCLANG_PATH".into(), clang.parent().unwrap().into()));
        let ff = ensure_ffmpeg(http, dirs)?;
        env.env.push(("FFMPEG_DIR".into(), ff.dir.clone().into()));
        env.ffmpeg = Some(ff);
    }
    Ok(env)
}

fn app_dir() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA isn't set")?;
    Ok(PathBuf::from(local).join(r"Programs\OpenQ"))
}

fn start_menu_link() -> Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA").context("APPDATA isn't set")?;
    Ok(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\OpenQ.lnk"))
}

/// The newest signtool.exe from the Windows SDK (installed with the C++ Build Tools).
fn signtool() -> Option<PathBuf> {
    let bin = program_files_x86().join(r"Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(bin).ok()?.flatten().map(|e| e.path().join(r"x64\signtool.exe")).filter(|p| p.is_file()).collect();
    versions.sort();
    versions.pop()
}

/// Signs `exe` with the certificate whose SHA-1 thumbprint is `thumbprint` (from the user's or
/// machine's certificate store, a smart card or a cloud HSM's local provider), with an RFC 3161
/// timestamp so the signature outlives the certificate.
fn sign(exe: &Path, thumbprint: &str) -> Result<()> {
    ui::step("Signing OpenQ.exe");
    let tool = signtool().context("signtool.exe not found (it comes with the Windows SDK)")?;
    util::run(Command::new(&tool).args(["sign", "/sha1", thumbprint, "/fd", "SHA256", "/tr", TIMESTAMP_URL, "/td", "SHA256"]).arg(exe))?;
    util::run(Command::new(&tool).args(["verify", "/pa"]).arg(exe))
}

fn powershell(script: &str, env: &[(&str, &Path)]) -> Result<()> {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().context("couldn't run PowerShell")?;
    if !out.status.success() {
        bail!("PowerShell: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Where oqman keeps itself, so "Installed apps" can run its uninstaller later.
fn own_copy(dirs: &Dirs) -> Result<PathBuf> {
    let dest = dirs.home.join(r"bin\oqman.exe");
    let me = std::env::current_exe()?;
    let same = std::fs::canonicalize(&me).ok() == std::fs::canonicalize(&dest).ok();
    if !same {
        util::copy(&me, &dest)?;
    }
    Ok(dest)
}

/// Puts OpenQ.exe and the FFmpeg DLLs in `%LOCALAPPDATA%\Programs\OpenQ`, adds the Start menu
/// shortcut and the "Installed apps" entry.
pub fn install(dirs: &Dirs, opts: &Options, built: &Built, _yes: bool) -> Result<Vec<PathBuf>> {
    ui::step("Installing");
    let dir = app_dir()?;
    let staging = dir.with_file_name("OpenQ.oqman");
    util::remove(&staging)?;
    let exe = staging.join("OpenQ.exe");
    util::copy(&built.exe, &exe)?;
    if let Some(ff) = &built.env.ffmpeg {
        for dll in std::fs::read_dir(ff.dir.join("bin"))?.flatten().map(|e| e.path()) {
            if dll.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
                util::copy(&dll, &staging.join(dll.file_name().unwrap()))?;
            }
        }
    }
    super::write_licences(built, &staging)?;
    if let Some(thumbprint) = &opts.sign {
        sign(&exe, thumbprint)?;
    }
    util::remove(&dir).context("couldn't replace the old installation; close OpenQ and try again")?;
    std::fs::rename(&staging, &dir)?;
    let exe = dir.join("OpenQ.exe");

    let link = start_menu_link()?;
    powershell(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:OQMAN_LINK); \
         $s.TargetPath = $env:OQMAN_TARGET; $s.WorkingDirectory = $env:OQMAN_DIR; $s.Save()",
        &[("OQMAN_LINK", &link), ("OQMAN_TARGET", &exe), ("OQMAN_DIR", &dir)],
    )?;

    let oqman = own_copy(dirs)?;
    let reg = |name: &str, kind: &str, value: &str| -> Result<()> {
        let out = Command::new("reg").args(["add", UNINSTALL_KEY, "/v", name, "/t", kind, "/d", value, "/f"]).output()?;
        if !out.status.success() {
            bail!("couldn't write the uninstall entry: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    };
    reg("DisplayName", "REG_SZ", "OpenQ")?;
    reg("DisplayVersion", "REG_SZ", built.version)?;
    reg("Publisher", "REG_SZ", "OpenQ")?;
    reg("DisplayIcon", "REG_SZ", &exe.display().to_string())?;
    reg("InstallLocation", "REG_SZ", &dir.display().to_string())?;
    reg("UninstallString", "REG_SZ", &format!("\"{}\" uninstall", oqman.display()))?;
    reg("NoModify", "REG_DWORD", "1")?;
    reg("NoRepair", "REG_DWORD", "1")?;
    ui::info(format!("OpenQ {} installed: {}", built.version, exe.display()));
    Ok(vec![dir, link])
}

/// Removes the "Installed apps" entry.
pub fn uninstall_extra() -> Result<()> {
    let _ = Command::new("reg").args(["delete", UNINSTALL_KEY, "/f"]).output();
    Ok(())
}

/// Prints the state of each dependency.
pub fn doctor(dirs: &Dirs, _opts: &Options) {
    ui::info(format!("C++ Build Tools: {}", msvc().unwrap_or_else(|| "missing".into())));
    ui::info(format!("libclang: {}", libclang(dirs).map(|p| p.display().to_string()).unwrap_or_else(|| "missing".into())));
    ui::info(format!("FFmpeg: {}", installed_ffmpeg(dirs).map(|p| p.display().to_string()).unwrap_or_else(|| "not downloaded yet".into())));
    ui::info(format!("signtool: {}", signtool().map(|p| p.display().to_string()).unwrap_or_else(|| "missing".into())));
}
