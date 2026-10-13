//! Linux: build dependencies come from the distribution's package manager (FFmpeg included);
//! the app goes to openqup's home with an `openq` link in `~/.local/bin` and a menu entry.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::{BuildEnv, Built, Wanted};
use crate::net::Http;
use crate::state::{Dirs, Options};
use crate::{ui, util};

/// Package families openqup knows how to install with.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Family {
    /// Debian, Ubuntu, Mint, Pop!_OS, Raspberry Pi OS... (apt)
    Debian,
    /// Fedora, RHEL, Rocky, Alma... (dnf)
    Fedora,
    /// Arch, Manjaro, EndeavourOS, CachyOS... (pacman)
    Arch,
    /// openSUSE (zypper)
    Suse,
}

/// The family from `/etc/os-release` (`ID` and `ID_LIKE`).
pub fn family(os_release: &str) -> Option<Family> {
    let mut ids = Vec::new();
    for line in os_release.lines() {
        if let Some(v) = line.strip_prefix("ID=").or_else(|| line.strip_prefix("ID_LIKE=")) {
            ids.extend(v.trim_matches('"').split_whitespace().map(str::to_ascii_lowercase));
        }
    }
    ids.iter().find_map(|id| match id.as_str() {
        "debian" | "ubuntu" => Some(Family::Debian),
        "fedora" | "rhel" | "centos" => Some(Family::Fedora),
        "arch" | "archlinux" => Some(Family::Arch),
        "suse" | "opensuse" | "sles" => Some(Family::Suse),
        id if id.starts_with("opensuse") => Some(Family::Suse),
        _ => None,
    })
}

/// One thing the build needs.
struct Need {
    what: &'static str,
    present: bool,
    /// Without it the build still works with fewer features (PipeWire).
    optional: bool,
    packages: Vec<&'static str>,
}

const FFMPEG_MODULES: [&str; 7] = ["libavcodec", "libavformat", "libavutil", "libavdevice", "libavfilter", "libswscale", "libswresample"];

fn packages(family: Family, what: &str) -> Vec<&'static str> {
    use Family::*;
    match (family, what) {
        (Debian, "C compiler") => vec!["build-essential"],
        (Fedora | Suse, "C compiler") => vec!["gcc"],
        (Arch, "C compiler") => vec!["base-devel"],
        (Debian, "libclang") => vec!["clang", "libclang-dev"],
        (Fedora | Suse, "libclang") => vec!["clang", "clang-devel"],
        (Arch, "libclang") => vec!["clang"],
        (Debian, "pkg-config") => vec!["pkg-config"],
        (Fedora | Arch, "pkg-config") => vec!["pkgconf"],
        (Suse, "pkg-config") => vec!["pkg-config"],
        (Debian, "ALSA") => vec!["libasound2-dev"],
        (Arch, "ALSA") => vec!["alsa-lib"],
        (Debian, "libudev") => vec!["libudev-dev"],
        (Arch, "libudev") => vec!["systemd-libs"],
        (Debian, "PipeWire") => vec!["libpipewire-0.3-dev"],
        (Arch, "PipeWire") => vec!["libpipewire"],
        (Debian, "FFmpeg") => vec![
            "libavcodec-dev", "libavformat-dev", "libavutil-dev", "libavdevice-dev", "libavfilter-dev", "libswscale-dev",
            "libswresample-dev",
        ],
        (Arch, "FFmpeg") => vec!["ffmpeg"],
        // dnf and zypper install by pkg-config name, whatever the package is called
        // (Fedora's ffmpeg-free-devel, RPM Fusion's ffmpeg-devel, openSUSE's ffmpeg-N-*-devel).
        (Fedora | Suse, "ALSA") => vec!["pkgconfig(alsa)"],
        (Fedora | Suse, "libudev") => vec!["pkgconfig(libudev)"],
        (Fedora | Suse, "PipeWire") => vec!["pkgconfig(libpipewire-0.3)"],
        (Fedora | Suse, "FFmpeg") => vec![
            "pkgconfig(libavcodec)", "pkgconfig(libavformat)", "pkgconfig(libavutil)", "pkgconfig(libavdevice)",
            "pkgconfig(libavfilter)", "pkgconfig(libswscale)", "pkgconfig(libswresample)",
        ],
        _ => Vec::new(),
    }
}

fn pkg_config(modules: &[&str]) -> bool {
    util::output(Command::new("pkg-config").arg("--exists").args(modules)).is_some()
}

/// libclang (bindgen loads it to read FFmpeg's headers).
fn find_libclang() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("LIBCLANG_PATH") {
        dirs.push(d.into());
    }
    if let Some(d) = util::output(Command::new("llvm-config").arg("--libdir")) {
        dirs.push(d.into());
    }
    for base in ["/usr/lib", "/usr/lib64", "/usr/local/lib", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu"] {
        dirs.push(base.into());
    }
    if let Ok(entries) = std::fs::read_dir("/usr/lib") {
        let mut llvm: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains("/llvm")).collect();
        llvm.sort();
        dirs.extend(llvm.into_iter().rev().map(|p| p.join("lib")));
    }
    dirs.into_iter().find_map(|d| {
        std::fs::read_dir(&d).ok()?.flatten().find(|e| e.file_name().to_string_lossy().starts_with("libclang.so")).map(|e| e.path())
    })
}

fn needs(wanted: &Wanted, family: Option<Family>) -> Vec<Need> {
    let pkgs = |what| family.map(|f| packages(f, what)).unwrap_or_default();
    let mut list = vec![
        Need { what: "C compiler", present: util::have("cc", "--version"), optional: false, packages: pkgs("C compiler") },
        Need { what: "pkg-config", present: util::have("pkg-config", "--version"), optional: false, packages: pkgs("pkg-config") },
        Need { what: "ALSA", present: pkg_config(&["alsa"]), optional: false, packages: pkgs("ALSA") },
        Need { what: "libudev", present: pkg_config(&["libudev"]), optional: false, packages: pkgs("libudev") },
    ];
    if wanted.pipewire {
        list.push(Need { what: "PipeWire", present: pkg_config(&["libpipewire-0.3"]), optional: true, packages: pkgs("PipeWire") });
    }
    if wanted.video {
        list.push(Need { what: "libclang", present: find_libclang().is_some(), optional: false, packages: pkgs("libclang") });
        list.push(Need { what: "FFmpeg", present: pkg_config(&FFMPEG_MODULES), optional: false, packages: pkgs("FFmpeg") });
    }
    list
}

fn install_command(family: Family, packages: &[&str]) -> Command {
    let mut cmd = Command::new("sudo");
    match family {
        Family::Debian => cmd.args(["apt-get", "install", "-y", "--no-install-recommends"]),
        Family::Fedora => cmd.args(["dnf", "install", "-y"]),
        Family::Arch => cmd.args(["pacman", "-S", "--needed", "--noconfirm"]),
        Family::Suse => cmd.args(["zypper", "--non-interactive", "install"]),
    };
    cmd.args(packages);
    cmd
}

fn os_family() -> Option<Family> {
    let text = std::fs::read_to_string("/etc/os-release").or_else(|_| std::fs::read_to_string("/usr/lib/os-release")).ok()?;
    family(&text)
}

/// Checks the build dependencies and installs missing ones with the package manager (asking
/// first, with sudo).
pub fn prepare(_dirs: &Dirs, _http: &Http, opts: &Options, yes: bool) -> Result<BuildEnv> {
    ui::step("Build dependencies");
    let wanted = Wanted::from(opts.no_default_features, &opts.features);
    let family = os_family();
    let mut list = needs(&wanted, family);
    let missing: Vec<&Need> = list.iter().filter(|n| !n.present).collect();
    if !missing.is_empty() {
        let names: Vec<_> = missing.iter().map(|n| n.what).collect();
        ui::info(format!("Missing: {}", names.join(", ")));
        let Some(family) = family else {
            bail!("Unknown Linux distribution: install {} with your package manager and run openqup again.", names.join(", "));
        };
        let pkgs: Vec<&str> = missing.iter().flat_map(|n| n.packages.iter().copied()).collect();
        let mut cmd = install_command(family, &pkgs);
        ui::info(format!("openqup will run: {}", util::show(&cmd)));
        if family == Family::Fedora && missing.iter().any(|n| n.what == "FFmpeg") {
            ui::info("(On RHEL-like systems FFmpeg comes from EPEL / RPM Fusion, which must be enabled.)");
        }
        if ui::confirm("Install these packages?", yes)? {
            if family == Family::Debian {
                util::run(Command::new("sudo").args(["apt-get", "update"]))?;
            }
            util::run(&mut cmd)?;
            list = needs(&wanted, Some(family));
        }
        let still: Vec<&Need> = list.iter().filter(|n| !n.present).collect();
        if still.iter().any(|n| !n.optional) {
            let names: Vec<_> = still.iter().map(|n| n.what).collect();
            bail!("Still missing: {}. Install them and run openqup again.", names.join(", "));
        }
    }
    let mut env = BuildEnv { no_default_features: opts.no_default_features, features: opts.features.clone(), ..Default::default() };
    if list.iter().any(|n| n.what == "PipeWire" && !n.present) {
        ui::warn("Building without PipeWire (ALSA only).");
        env.no_default_features = true;
        env.features.retain(|f| f != "pipewire");
        if wanted.video && !env.features.iter().any(|f| f == "video") {
            env.features.push("video".into());
        }
    }
    if let Some(lib) = find_libclang() {
        env.env.push(("LIBCLANG_PATH".into(), lib.parent().unwrap_or(Path::new("/")).into()));
    }
    ui::info("All found.");
    Ok(env)
}

fn data_home() -> Result<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        Some(d) => Ok(d.into()),
        None => Ok(util::home_dir()?.join(".local/share")),
    }
}

/// Puts the program in `<home>/app`, links `~/.local/bin/openq` to it and adds a menu entry.
pub fn install(dirs: &Dirs, _opts: &Options, built: &Built, _yes: bool) -> Result<Vec<PathBuf>> {
    ui::step("Installing");
    let app = dirs.home.join("app");
    let staging = dirs.home.join("app.new");
    util::remove(&staging)?;
    util::copy(&built.exe, &staging.join("openq"))?;
    super::write_licences(built, &staging.join("licenses"))?;
    util::remove(&app)?;
    std::fs::rename(&staging, &app)?;
    let exe = app.join("openq");
    let mut installed = vec![app.clone()];

    let link = util::home_dir()?.join(".local/bin/openq");
    let ours = std::fs::read_link(&link).is_ok_and(|t| t == exe);
    if std::fs::symlink_metadata(&link).is_ok() && !ours {
        ui::warn(format!("{} exists and isn't openqup's link; left as is", link.display()));
    } else {
        util::remove(&link)?;
        std::fs::create_dir_all(link.parent().unwrap())?;
        std::os::unix::fs::symlink(&exe, &link).with_context(|| format!("couldn't create {}", link.display()))?;
        installed.push(link.clone());
        let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == link.parent().unwrap()));
        if !on_path {
            ui::warn(format!("{} isn't on your PATH; start OpenQ from the menu or with {}", link.parent().unwrap().display(), exe.display()));
        }
    }

    let desktop = data_home()?.join("applications/openq.desktop");
    std::fs::create_dir_all(desktop.parent().unwrap())?;
    std::fs::write(
        &desktop,
        format!(
            "[Desktop Entry]\nType=Application\nName=OpenQ\nGenericName=Show control\n\
             Comment=Cue list playback for theatre and live events\nExec=\"{}\" %f\nIcon=applications-multimedia\n\
             Terminal=false\nCategories=AudioVideo;Audio;Video;\nStartupWMClass=openq\n",
            exe.display()
        ),
    )?;
    installed.push(desktop);
    ui::info(format!("OpenQ {} installed: {}", built.version, exe.display()));
    Ok(installed)
}

/// Nothing outside the recorded files on Linux.
pub fn uninstall_extra() -> Result<()> {
    Ok(())
}

/// Prints the state of each dependency.
pub fn doctor(_dirs: &Dirs, opts: &Options) {
    let family = os_family();
    ui::info(format!("Distribution family: {}", family.map(|f| format!("{f:?}")).unwrap_or_else(|| "unknown".into())));
    for n in needs(&Wanted::from(opts.no_default_features, &opts.features), family) {
        ui::info(format!("{:<11} {}", n.what, if n.present { "ok" } else if n.optional { "missing (optional)" } else { "missing" }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families() {
        assert_eq!(family("NAME=\"CachyOS Linux\"\nID=cachyos\nID_LIKE=arch\n"), Some(Family::Arch));
        assert_eq!(family("ID=ubuntu\nID_LIKE=debian\n"), Some(Family::Debian));
        assert_eq!(family("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"), Some(Family::Fedora));
        assert_eq!(family("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"), Some(Family::Suse));
        assert_eq!(family("ID=nixos\n"), None);
    }

    #[test]
    fn every_family_has_packages() {
        for f in [Family::Debian, Family::Fedora, Family::Arch, Family::Suse] {
            for what in ["C compiler", "pkg-config", "ALSA", "libudev", "PipeWire", "libclang", "FFmpeg"] {
                assert!(!packages(f, what).is_empty(), "{f:?} {what}");
            }
        }
    }
}
