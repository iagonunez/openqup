//! What differs per system: build dependencies, where the app goes, shortcuts and signing.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::util;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// What the build needs from the dependencies step.
#[derive(Default)]
pub struct BuildEnv {
    /// Extra environment variables (`FFMPEG_DIR`, `LIBCLANG_PATH`, ...).
    pub env: Vec<(String, OsString)>,
    /// Folders to put first on PATH.
    pub path: Vec<PathBuf>,
    /// Build without the default features (then only `features`).
    pub no_default_features: bool,
    /// Cargo features to turn on.
    pub features: Vec<String>,
    /// The FFmpeg that gets bundled with the app (macOS / Windows), if any.
    pub ffmpeg: Option<Ffmpeg>,
}

/// A bundled FFmpeg build.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub struct Ffmpeg {
    /// Install prefix (with `bin/` or `lib/`).
    pub dir: PathBuf,
    /// Its licence and build files, copied next to the app.
    pub licence_files: Vec<PathBuf>,
    /// Where it came from, for the licence folder's README.
    pub origin: String,
}

/// Which optional parts the build asks for.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct Wanted {
    /// The `video` feature (FFmpeg).
    pub video: bool,
    /// The `pipewire` feature (Linux).
    pub pipewire: bool,
}

impl Wanted {
    /// From the user's feature choice (the defaults are `video` and `pipewire`).
    pub fn from(no_default_features: bool, features: &[String]) -> Self {
        let has = |f: &str| features.iter().any(|x| x == f);
        Self { video: !no_default_features || has("video"), pipewire: !no_default_features || has("pipewire") }
    }
}

/// A finished build, ready to put in place.
pub struct Built<'a> {
    /// The compiled program.
    pub exe: PathBuf,
    /// The source tree (for licence files).
    pub src: &'a Path,
    /// OpenQ's version.
    pub version: &'a str,
    /// The dependencies it was built with.
    pub env: &'a BuildEnv,
}

/// Copies OpenQ's licence files (and the bundled FFmpeg's) into `dir`, as the licences of
/// OpenQ, its fonts and FFmpeg ask for.
pub fn write_licences(built: &Built, dir: &Path) -> Result<()> {
    for f in ["LICENSE.md", "NOTICE.md", "THIRD_PARTY_LICENSES.md"] {
        let p = built.src.join(f);
        if p.is_file() {
            util::copy(&p, &dir.join(f))?;
        }
    }
    let lic = dir.join("Licenses");
    if let Ok(fonts) = std::fs::read_dir(built.src.join("assets/fonts")) {
        for f in fonts.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "txt")) {
            util::copy(&f, &lic.join("fonts").join(f.file_name().unwrap()))?;
        }
    }
    if let Some(ff) = &built.env.ffmpeg {
        let ffdir = lic.join("ffmpeg");
        for f in &ff.licence_files {
            util::copy(f, &ffdir.join(f.file_name().unwrap()))?;
        }
        std::fs::write(ffdir.join("README.md"), ffmpeg_readme(&ff.origin))?;
    }
    Ok(())
}

fn ffmpeg_readme(origin: &str) -> String {
    format!(
        "# FFmpeg in this OpenQ installation\n\n\
         OpenQ plays video with the FFmpeg libraries (libavcodec, libavformat, libavutil, libavdevice,\n\
         libavfilter, libswscale, libswresample). oqman installed them next to OpenQ as separate\n\
         shared libraries.\n\n\
         - **Licence:** GNU Lesser General Public License 2.1 or later (the licence file next to this\n  \
         one). Copyright the FFmpeg developers, <https://ffmpeg.org>.\n\
         - **Build:** {origin}\n\
         - **LGPL-only:** configured without GPL, version-3 or nonfree components; oqman refuses\n  \
         any other build.\n\
         - **Source:** <https://ffmpeg.org/download.html> and <https://github.com/FFmpeg/FFmpeg>.\n\
         - **Replacing them:** the libraries are loaded dynamically, so you may swap in your own build\n  \
         of the same major version.\n"
    )
}

/// Seconds since 1970, for the state file.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
