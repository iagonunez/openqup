# oqman

Installs [OpenQ](https://github.com/iagonunez/OpenQ) by building it from source on your computer,
the way [rustup](https://rustup.rs) installs Rust. oqman:

1. picks the OpenQ version (the newest `vX.Y.Z` tag, or the one you ask for) and downloads its
   source;
2. finds a Rust toolchain, or installs a private one with the official `rustup-init`;
3. installs the build dependencies for your system (C compiler, libclang, FFmpeg, audio headers);
4. builds OpenQ (`cargo build --release --locked`);
5. installs the app with a menu entry, signs it on macOS (and on Windows if you have a
   certificate), and remembers how, so `oqman update` can do it again.

Nothing is installed system-wide except the packages your Linux package manager installs (after
asking). oqman keeps its toolchain, dependencies and build cache in its own folder, and
`oqman uninstall --purge` removes all of it.

## Install

**Linux and macOS:**

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://github.com/iagonunez/oqman/releases/latest/download/install.sh | sh
```

**Windows** (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/iagonunez/oqman/releases/latest/download/install.ps1 | iex"
```

The bootstrap scripts download the oqman binary for your system, check it against the
release's `SHA256SUMS` (and its signature), put it in `~/.local/bin` (Windows:
`%LOCALAPPDATA%\oqman\bin`, added to your PATH) and run `oqman install`.

If you already have Rust: `cargo install --locked --git https://github.com/iagonunez/oqman`, then
`oqman install`.

## Use

```text
oqman install [options]       build and install OpenQ (newest release unless --ref)
oqman update [-y] [--force]   rebuild when the followed version or oqman itself has changed
oqman uninstall [--purge]     remove the app (--purge: also toolchain, dependencies, caches)
oqman versions                list the versions that can be installed
oqman doctor                  show what is installed and what the build needs
```

Install options: `--ref <tag|branch|commit>`, `--repo <owner/name>`, `--git <url>`,
`--source <dir>` (build a local checkout), `--features a,b`, `--no-default-features`,
`--no-lighting` (cues only: no Lighting view, Light cues, effect scripts or fixture downloads, and no
Python download; `--lighting` brings it back),
`--system-ffmpeg` (macOS, use Homebrew's), `--sign <identity>`, `--notarize <profile>`, `-y`.
`update` keeps the options of the last install unless you give new ones.

| | App | oqman's folder (`OQMAN_HOME`) |
|---|---|---|
| Linux | `~/.local/share/oqman/app/openq`, linked as `~/.local/bin/openq`, menu entry in `~/.local/share/applications` | `~/.local/share/oqman` |
| macOS | `~/Applications/OpenQ.app` (FFmpeg inside) | `~/Library/Application Support/oqman` |
| Windows | `%LOCALAPPDATA%\Programs\OpenQ` (FFmpeg DLLs next to `OpenQ.exe`), Start menu shortcut, entry in *Installed apps* | `%LOCALAPPDATA%\oqman` |

## What is downloaded, from where, and under which licence

Every download comes from the project's official site over HTTPS. Archives are checked against
a SHA-256 pinned in oqman or published by the project; the Microsoft installer, which changes
with each update, is checked for a valid Microsoft Authenticode signature instead.

| Component | Systems | Source | Check | Licence |
|---|---|---|---|---|
| OpenQ source | all | GitHub (`api.github.com/.../tarball/<commit>` or `git fetch`) | exact commit id | Apache-2.0 |
| Rust (rustup-init, stable toolchain) | all, if missing | `static.rust-lang.org` | published `.sha256` | MIT / Apache-2.0 |
| Crates (OpenQ's dependencies) | all | crates.io, through cargo | `Cargo.lock` checksums | their own (listed in OpenQ's `THIRD_PARTY_LICENSES.md`) |
| C compiler, clang, ALSA, udev, PipeWire, FFmpeg | Linux | your distribution's package manager (asks first, uses sudo) | the package manager's signatures | their own |
| Xcode Command Line Tools | macOS, if missing | Apple, through `xcode-select --install` (Apple's installer; you accept Apple's licence there) | Apple | Apple's licence |
| FFmpeg 9.0.2 source | macOS | `ffmpeg.org/releases` | pinned SHA-256 | LGPL-2.1+ (built with GPL, version-3 and nonfree parts off; oqman stops if the configuration says otherwise) |
| NASM 3.02 source | Intel Macs without nasm | `nasm.us` | pinned SHA-256 | BSD-2-Clause |
| Visual Studio Build Tools (C++) | Windows, if missing | Microsoft (`aka.ms/vs/17/release/vs_BuildTools.exe`), opened in Microsoft's installer where you accept Microsoft's licence | Microsoft Authenticode signature | Microsoft's licence |
| LLVM 23.1.3 (libclang only) | Windows, if missing | GitHub `llvm/llvm-project` release (MSI unpacked, not installed) | pinned SHA-256 | Apache-2.0 WITH LLVM-exception |
| FFmpeg 9.0 shared, LGPL | Windows | GitHub `BtbN/FFmpeg-Builds` (`lgpl-shared` only; oqman refuses other variants) | GitHub's SHA-256 digest | LGPL-2.1+ |
| CPython 3.15.0 for WebAssembly (WASI) | all (OpenQ with lighting effect scripts) | GitHub `brettcannon/cpython-wasi-build` release (built by a CPython core developer) | pinned SHA-256 | PSF License |

Notes:

- The Python is **not** a normal Python: OpenQ runs it inside its own WebAssembly sandbox
  (wasmtime) only to generate lighting effects. It can't read or write your files, use the
  network or start programs. oqman installs `python.wasm` and only the computation part of the
  standard library (about 2.6 MB) as `python/` next to the app, with its licence in the app's
  licence folder.

- oqman never accepts a licence on your behalf where the vendor asks for acceptance (Apple,
  Microsoft): it opens the vendor's own installer. `-y` only answers oqman's own questions.
- Building for yourself isn't distribution, so OpenQ's licence and the LGPL ask nothing more of
  you. The FFmpeg licence, build details and OpenQ's notices are still installed with the app
  (`Licenses/`). If you copy the built app to others, you are distributing it: keep those files
  with it, and offer the FFmpeg source (the LGPL requires it). The Linux build uses your
  distribution's FFmpeg, which is often GPL: fine for your own use; a copy passed on falls under
  GPL 3 as a whole, with its complete source (see OpenQ's `NOTICE.md`).

## Code signing

**The app oqman builds.** Software built on your own computer has no quarantine flag (macOS)
or Mark of the Web (Windows), so Gatekeeper and SmartScreen don't block it, and no certificate is
needed to run it.

- **macOS:** the app is always signed, ad hoc by default (Apple silicon requires a signature;
  oqman signs after placing FFmpeg in the bundle). With an Apple Developer ID certificate in
  your keychain: `oqman install --sign "Developer ID Application: Name (TEAMID)"` signs with
  the hardened runtime, a timestamp and microphone/camera entitlements. Add
  `--notarize <profile>` (made with `xcrun notarytool store-credentials <profile>`) to notarise
  and staple it, so the app also opens cleanly when copied to other Macs.
- **Windows:** `oqman install --sign <SHA-1 thumbprint>` signs `OpenQ.exe` with that
  certificate from your certificate store (or a token / cloud HSM with a local CSP) using
  `signtool` from the Windows SDK, with an RFC 3161 timestamp. Useful where policies (AppLocker,
  WDAC, Smart App Control) only allow signed programs.

**oqman itself.** The release workflow signs the oqman binaries when the secrets are set (see
the top of `.github/workflows/release.yml`):

- macOS: Developer ID certificate (Apple Developer Program, 99 USD/year), hardened runtime,
  notarised.
- Windows: Azure Artifact Signing (formerly Trusted Signing; the cheapest option for
  individuals) or any Authenticode certificate (`.pfx`).

Without the secrets the binaries are published unsigned; the bootstrap scripts still verify
their SHA-256 sums.

## Development

```sh
cargo clippy --all-targets          # also with --target aarch64-apple-darwin / x86_64-pc-windows-msvc
cargo test
cargo run -- doctor
OQMAN_HOME=/tmp/oq cargo run -- install --source ../openq -y    # try it on a local checkout
python3 -I tools/third_party_licenses.py                            # after dependency changes
```

- `.github/workflows/release.yml`: tests on push; a `v*` tag builds, signs and publishes the
  binaries with `SHA256SUMS`, `install.sh` and `install.ps1`.
- `.github/workflows/e2e.yml` (run by hand): installs OpenQ from scratch on Linux, macOS (arm64
  and Intel) and Windows runners, checks the result and uninstalls it.
- Pinned versions and checksums (FFmpeg, NASM, LLVM) are constants at the top of
  `src/platform/macos.rs` and `src/platform/windows.rs`; the FFmpeg series must match what
  OpenQ's `ffmpeg-next` supports. The sandboxed Python is pinned in `src/python.rs` (keep it and
  its module list in step with OpenQ's `tools/fetch_python_wasi.py`).

## Licence

Apache License 2.0 (`LICENSE`). Third-party crates: `THIRD_PARTY_LICENSES.md`.
