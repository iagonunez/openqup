//! The sandboxed Python that OpenQ's lighting effect scripts run in.
//!
//! OpenQ runs effect scripts in CPython built for WebAssembly (WASI) inside its own wasmtime
//! sandbox: no files besides a read-only standard library, no network, no other programs, capped
//! memory and run time. oqman installs that interpreter as `python/` next to the app: the
//! `python.wasm` module, its licence and only the part of the standard library scripts need.
//! It is pinned by version and SHA-256 (builds by a CPython core developer,
//! <https://github.com/brettcannon/cpython-wasi-build>).
//!
//! Keep `VERSION`, `URL`, `SHA256` and `KEEP` in step with OpenQ's `tools/fetch_python_wasi.py`.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::net::{Http, Sum};
use crate::state::Dirs;
use crate::{ui, util};

const VERSION: &str = "3.15.0";
const URL: &str = "https://github.com/brettcannon/cpython-wasi-build/releases/download/v3.15.0/python-3.15.0-wasi_sdk-33.zip";
const SHA256: &str = "00b791b3729230f97a625087f534d71978433fb1e068e13a9cac8cd3f2541742";
const LIB: &str = "lib/python3.15/";

/// Standard library modules (files or packages) kept; the rest is left out.
const KEEP: &str = "
__future__ _collections_abc _colorize _compat_pickle _opcode_metadata _py_abc _py_warnings _pydatetime
_pydecimal _pylong _sitebuiltins _strptime _sysconfigdata__wasi_wasm32-wasi _weakrefset abc annotationlib ast
bisect calendar codecs codeop collections colorsys contextlib contextvars copy copyreg dataclasses datetime decimal
dis enum fractions functools genericpath heapq inspect io itertools json keyword linecache locale numbers
opcode operator os posixpath pprint random re reprlib stat statistics string struct textwrap token tokenize
traceback types typing warnings weakref importlib pathlib
encodings/__init__ encodings/aliases encodings/utf_8 encodings/utf_8_sig encodings/ascii encodings/latin_1
";

/// Whether the OpenQ source uses effect scripts (and so wants the interpreter).
pub fn wanted(src: &Path, no_default_features: bool, features: &[String]) -> bool {
    let has_feature = std::fs::read_to_string(src.join("Cargo.toml")).is_ok_and(|t| t.contains("\nscripting = ["));
    has_feature && (!no_default_features || features.iter().any(|f| f == "scripting" || f == "lighting"))
}

/// Download (or reuse) the interpreter and unpack the kept files into `<home>/python-wasi/<version>`.
pub fn prepare(dirs: &Dirs, http: &Http) -> Result<PathBuf> {
    ui::step(format!("Python {VERSION} for effect scripts (WebAssembly sandbox)"));
    let archive = dirs.downloads().join(format!("python-{VERSION}-wasi.zip"));
    http.download(URL, &archive, Sum::Sha256(SHA256))?;
    let dest = dirs.home.join("python-wasi").join(VERSION);
    if dest.join("python.wasm").is_file() && dest.join(".complete").is_file() {
        return Ok(dest);
    }
    let staging = dest.with_extension("new");
    util::remove(&staging)?;
    let bytes = std::fs::read(&archive)?;
    let mut n = 0;
    for entry in zip_entries(&bytes)? {
        let name = entry.name.as_str();
        let keep = name == "python.wasm" || name == "LICENSE" || name.strip_prefix(LIB).is_some_and(kept);
        if !keep || name.ends_with('/') {
            continue;
        }
        let path = staging.join(name);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, entry.data(&bytes)?).with_context(|| format!("couldn't write {}", path.display()))?;
        n += 1;
    }
    if !staging.join("python.wasm").is_file() {
        bail!("{URL} has no python.wasm");
    }
    // Python looks for this folder; it stays empty (no native extension modules).
    std::fs::create_dir_all(staging.join(LIB).join("lib-dynload"))?;
    std::fs::write(staging.join(".complete"), "")?;
    util::remove(&dest)?;
    std::fs::rename(&staging, &dest)?;
    ui::info(format!("{n} files"));
    Ok(dest)
}

/// Copy a prepared interpreter to `dest` (the app's `python` folder).
pub fn install(from: &Path, dest: &Path) -> Result<()> {
    copy_tree(from, dest)
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let (src, name) = (e.path(), e.file_name());
        if name == ".complete" {
            continue;
        }
        if src.is_dir() {
            copy_tree(&src, &to.join(&name))?;
        } else {
            util::copy(&src, &to.join(&name))?;
        }
    }
    Ok(())
}

/// The licence file for the app's licence folder, and its README.
pub fn licence_readme() -> String {
    format!(
        "# Python in this OpenQ installation\n\n\
         OpenQ runs lighting effect scripts in CPython {VERSION} compiled to WebAssembly (WASI), inside\n\
         its own sandbox (no files besides the read-only standard library, no network, no other\n\
         programs). oqman installed `python.wasm` and part of the standard library in the `python`\n\
         folder next to OpenQ.\n\n\
         - **Licence:** Python Software Foundation License (the LICENSE file next to this one).\n\
         - **Build:** {URL}\n  (SHA-256 {SHA256})\n\
         - **Source:** <https://www.python.org/> and <https://github.com/python/cpython>.\n"
    )
}

/// Whether a path inside `lib/python3.15/` is kept.
fn kept(rel: &str) -> bool {
    if rel.contains("__pycache__") || rel.contains("/test") || rel.starts_with("test") {
        return false;
    }
    let stem = rel.strip_suffix(".py").unwrap_or(rel);
    KEEP.split_whitespace().any(|k| {
        if k.contains('/') {
            stem == k
        } else {
            stem == k || (rel.starts_with(&format!("{k}/")) && !rel.starts_with("encodings/"))
        }
    })
}

/// One file in a zip archive.
struct Entry {
    name: String,
    method: u16,
    offset: usize,
    compressed: usize,
    size: usize,
}

impl Entry {
    /// The file's contents (stored or deflated).
    fn data(&self, zip: &[u8]) -> Result<Vec<u8>> {
        let h = zip.get(self.offset..self.offset + 30).context("zip: bad local header")?;
        if u32_at(h, 0) != 0x0403_4b50 {
            bail!("zip: bad local header for {}", self.name);
        }
        let start = self.offset + 30 + u16_at(h, 26) as usize + u16_at(h, 28) as usize;
        let raw = zip.get(start..start + self.compressed).context("zip: truncated")?;
        let out = match self.method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(self.size);
                flate2::read::DeflateDecoder::new(raw).take(self.size as u64 + 1).read_to_end(&mut out)?;
                out
            }
            m => bail!("zip: {} uses compression method {m}", self.name),
        };
        if out.len() != self.size {
            bail!("zip: {} has the wrong size", self.name);
        }
        Ok(out)
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The entries of a zip archive, from its central directory. Names that could escape the
/// destination (absolute, `..`, backslashes) are refused.
fn zip_entries(zip: &[u8]) -> Result<Vec<Entry>> {
    let tail = zip.len().saturating_sub(65_557);
    let eocd = (tail..zip.len().saturating_sub(21)).rev().find(|&i| u32_at(zip, i) == 0x0605_4b50).context("zip: no end of central directory")?;
    let count = u16_at(zip, eocd + 10) as usize;
    let mut pos = u32_at(zip, eocd + 16) as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let h = zip.get(pos..pos + 46).context("zip: truncated central directory")?;
        if u32_at(h, 0) != 0x0201_4b50 {
            bail!("zip: bad central directory");
        }
        let (name_len, extra, comment) = (u16_at(h, 28) as usize, u16_at(h, 30) as usize, u16_at(h, 32) as usize);
        let name = String::from_utf8(zip.get(pos + 46..pos + 46 + name_len).context("zip: truncated")?.to_vec())?;
        if name.starts_with('/') || name.contains('\\') || name.split('/').any(|p| p == "..") {
            bail!("zip: unsafe path {name}");
        }
        entries.push(Entry {
            name,
            method: u16_at(h, 10),
            compressed: u32_at(h, 20) as usize,
            size: u32_at(h, 24) as usize,
            offset: u32_at(h, 42) as usize,
        });
        pos += 46 + name_len + extra + comment;
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_files() {
        assert!(kept("json/decoder.py"));
        assert!(kept("random.py"));
        assert!(kept("encodings/utf_8.py"));
        assert!(!kept("encodings/cp1252.py"));
        assert!(!kept("subprocess.py"));
        assert!(!kept("json/__pycache__/x.pyc"));
        assert!(!kept("test/test_json.py"));
        assert!(!kept("osx.py"), "prefix of a kept name");
    }

    /// A zip made by hand: one stored entry, one deflated entry, one unsafe name.
    #[test]
    fn reads_zip_files() {
        use std::io::Write;
        fn zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
            let (mut out, mut central) = (Vec::new(), Vec::new());
            for (name, data, deflate) in files {
                let body = if *deflate {
                    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                    e.write_all(data).unwrap();
                    e.finish().unwrap()
                } else {
                    data.to_vec()
                };
                let offset = out.len() as u32;
                let method: u16 = if *deflate { 8 } else { 0 };
                out.extend(0x0403_4b50u32.to_le_bytes());
                out.extend([20, 0, 0, 0]);
                out.extend(method.to_le_bytes());
                out.extend([0; 8]); // time, date, crc
                out.extend((body.len() as u32).to_le_bytes());
                out.extend((data.len() as u32).to_le_bytes());
                out.extend((name.len() as u16).to_le_bytes());
                out.extend([0, 0]);
                out.extend(name.as_bytes());
                out.extend(&body);
                central.extend(0x0201_4b50u32.to_le_bytes());
                central.extend([20, 0, 20, 0, 0, 0]);
                central.extend(method.to_le_bytes());
                central.extend([0; 8]);
                central.extend((body.len() as u32).to_le_bytes());
                central.extend((data.len() as u32).to_le_bytes());
                central.extend((name.len() as u16).to_le_bytes());
                central.extend([0; 12]);
                central.extend(offset.to_le_bytes());
                central.extend(name.as_bytes());
            }
            let cd = out.len() as u32;
            out.extend(&central);
            out.extend(0x0605_4b50u32.to_le_bytes());
            out.extend([0; 4]);
            out.extend((files.len() as u16).to_le_bytes());
            out.extend((files.len() as u16).to_le_bytes());
            out.extend((central.len() as u32).to_le_bytes());
            out.extend(cd.to_le_bytes());
            out.extend([0, 0]);
            out
        }
        let z = zip(&[("a.txt", b"hello", false), ("lib/b.py", &b"print(1)\n".repeat(50), true)]);
        let e = zip_entries(&z).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].data(&z).unwrap(), b"hello");
        assert_eq!(e[1].data(&z).unwrap(), b"print(1)\n".repeat(50));
        assert!(zip_entries(&zip(&[("../evil", b"x", false)])).is_err());
        assert!(zip_entries(&zip(&[("/abs", b"x", false)])).is_err());
    }
}

#[cfg(test)]
mod archive_test {
    /// With `OQMAN_PYTHON_ZIP` pointing at the downloaded archive: every kept file unpacks.
    #[test]
    fn real_archive() {
        let Some(p) = std::env::var_os("OQMAN_PYTHON_ZIP") else { return };
        let z = std::fs::read(p).unwrap();
        let kept: Vec<_> = super::zip_entries(&z).unwrap().into_iter().filter(|e| e.name == "python.wasm" || e.name.strip_prefix(super::LIB).is_some_and(super::kept)).collect();
        let total: usize = kept.iter().map(|e| e.data(&z).unwrap().len()).sum();
        println!("{} files, {} bytes", kept.len(), total);
        assert!(kept.len() > 90);
    }
}
