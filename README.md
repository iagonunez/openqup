# openqup

Installs [OpenQ](https://github.com/iagonunez/OpenQ) by building it from source on your computer,
the way [rustup](https://rustup.rs) installs Rust. openqup:

1. picks the OpenQ version (the newest `vX.Y.Z` tag, or the one you ask for) and downloads its
   source;
2. finds a Rust toolchain, or installs a private one with the official `rustup-init`;
3. installs the build dependencies for your system (C compiler, libclang, FFmpeg, audio headers);
4. builds OpenQ (`cargo build --release --locked`);
5. installs the app with a menu entry, signs it on macOS (and on Windows if you have a
   certificate), and remembers how, so `openqup update` can do it again.

Nothing is installed system-wide except the packages your Linux package manager installs (after
asking). openqup keeps its toolchain, dependencies and build cache in its own folder, and
`openqup uninstall --purge` removes all of it.

## Install

**Linux and macOS:**

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://github.com/iagonunez/openqup/releases/latest/download/install.sh | sh
```

**Windows** (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/iagonunez/openqup/releases/latest/download/install.ps1 | iex"
```

The bootstrap scripts download the openqup binary for your system, check it against the
release's `SHA256SUMS` (and its signature), put it in `~/.local/bin` (Windows:
`%LOCALAPPDATA%\openqup\bin`, added to your PATH) and run `openqup install`.

If you already have Rust: `cargo install --locked --git https://github.com/iagonunez/openqup`, then
`openqup install`.

## Use

```text
openqup install [options]       build and install OpenQ (newest release unless --ref)
openqup update [-y] [--force]   rebuild when the followed version has moved
openqup uninstall [--purge]     remove the app (--purge: also toolchain, dependencies, caches)
openqup versions                list the versions that can be installed
openqup doctor                  show what is installed and what the build needs
```

Install options: `--ref <tag|branch|commit>`, `--repo <owner/name>`, `--git <url>`,
`--source <dir>` (build a local checkout), `--features a,b`, `--no-default-features`,
`--system-ffmpeg` (macOS, use Homebrew's), `--sign <identity>`, `--notarize <profile>`, `-y`.
`update` keeps the options of the last install unless you give new ones.

| | App | openqup's folder (`OPENQUP_HOME`) |
|---|---|---|
| Linux | `~/.local/share/openqup/app/openq`, linked as `~/.local/bin/openq`, menu entry in `~/.local/share/applications` | `~/.local/share/openqup` |
| macOS | `~/Applications/OpenQ.app` (FFmpeg inside) | `~/Library/Application Support/openqup` |
| Windows | `%LOCALAPPDATA%\Programs\OpenQ` (FFmpeg DLLs next to `OpenQ.exe`), Start menu shortcut, entry in *Installed apps* | `%LOCALAPPDATA%\openqup` |

## What is downloaded, from where, and under which licence

Every download comes from the project's official site over HTTPS. Archives are checked against
a SHA-256 pinned in openqup or published by the project; the Microsoft installer, which changes
with each update, is checked for a valid Microsoft Authenticode signature instead.

| Component | Systems | Source | Check | Licence |
|---|---|---|---|---|
| OpenQ source | all | GitHub (`api.github.com/.../tarball/<commit>` or `git fetch`) | exact commit id | Apache-2.0 + Commons Clause |
| Rust (rustup-init, stable toolchain) | all, if missing | `static.rust-lang.org` | published `.sha256` | MIT / Apache-2.0 |
| Crates (OpenQ's dependencies) | all | crates.io, through cargo | `Cargo.lock` checksums | their own (listed in OpenQ's `THIRD_PARTY_LICENSES.md`) |
| C compiler, clang, ALSA, udev, PipeWire, FFmpeg | Linux | your distribution's package manager (asks first, uses sudo) | the package manager's signatures | their own |
| Xcode Command Line Tools | macOS, if missing | Apple, through `xcode-select --install` (Apple's installer; you accept Apple's licence there) | Apple | Apple's licence |
| FFmpeg 9.0.2 source | macOS | `ffmpeg.org/releases` | pinned SHA-256 | LGPL-2.1+ (built with GPL, version-3 and nonfree parts off; openqup stops if the configuration says otherwise) |
| NASM 3.02 source | Intel Macs without nasm | `nasm.us` | pinned SHA-256 | BSD-2-Clause |
| Visual Studio Build Tools (C++) | Windows, if missing | Microsoft (`aka.ms/vs/17/release/vs_BuildTools.exe`), opened in Microsoft's installer where you accept Microsoft's licence | Microsoft Authenticode signature | Microsoft's licence |
| LLVM 23.1.3 (libclang only) | Windows, if missing | GitHub `llvm/llvm-project` release (MSI unpacked, not installed) | pinned SHA-256 | Apache-2.0 WITH LLVM-exception |
| FFmpeg 9.0 shared, LGPL | Windows | GitHub `BtbN/FFmpeg-Builds` (`lgpl-shared` only; openqup refuses other variants) | GitHub's SHA-256 digest | LGPL-2.1+ |

Notes:

- openqup never accepts a licence on your behalf where the vendor asks for acceptance (Apple,
  Microsoft): it opens the vendor's own installer. `-y` only answers openqup's own questions.
- Building for yourself isn't distribution, so OpenQ's licence and the LGPL ask nothing more of
  you. The FFmpeg licence, build details and OpenQ's notices are still installed with the app
  (`Licenses/`). If you copy the built app to others, you are distributing it: keep those files
  with it, and offer the FFmpeg source (the LGPL requires it). The Linux build uses your
  distribution's FFmpeg, which is often GPL: fine for your own use, but don't redistribute such a
  build (see OpenQ's `NOTICE.md`).

## Code signing

**The app openqup builds.** Software built on your own computer has no quarantine flag (macOS)
or Mark of the Web (Windows), so Gatekeeper and SmartScreen don't block it, and no certificate is
needed to run it.

- **macOS:** the app is always signed, ad hoc by default (Apple silicon requires a signature;
  openqup signs after placing FFmpeg in the bundle). With an Apple Developer ID certificate in
  your keychain: `openqup install --sign "Developer ID Application: Name (TEAMID)"` signs with
  the hardened runtime, a timestamp and microphone/camera entitlements. Add
  `--notarize <profile>` (made with `xcrun notarytool store-credentials <profile>`) to notarise
  and staple it, so the app also opens cleanly when copied to other Macs.
- **Windows:** `openqup install --sign <SHA-1 thumbprint>` signs `OpenQ.exe` with that
  certificate from your certificate store (or a token / cloud HSM with a local CSP) using
  `signtool` from the Windows SDK, with an RFC 3161 timestamp. Useful where policies (AppLocker,
  WDAC, Smart App Control) only allow signed programs.

**openqup itself.** The release workflow signs the openqup binaries when the secrets are set (see
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
OPENQUP_HOME=/tmp/oq cargo run -- install --source ../openq -y    # try it on a local checkout
python3 -I tools/third_party_licenses.py                            # after dependency changes
```

- `.github/workflows/release.yml`: tests on push; a `v*` tag builds, signs and publishes the
  binaries with `SHA256SUMS`, `install.sh` and `install.ps1`.
- `.github/workflows/e2e.yml` (run by hand): installs OpenQ from scratch on Linux, macOS (arm64
  and Intel) and Windows runners, checks the result and uninstalls it.
- Pinned versions and checksums (FFmpeg, NASM, LLVM) are constants at the top of
  `src/platform/macos.rs` and `src/platform/windows.rs`; the FFmpeg series must match what
  OpenQ's `ffmpeg-next` supports.

## Licence

Apache License 2.0 (`LICENSE`). Third-party crates: `THIRD_PARTY_LICENSES.md`.
