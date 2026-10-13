#!/bin/sh
# openqup bootstrap for Linux and macOS.
#
#   curl --proto '=https' --tlsv1.2 -fsSL https://github.com/iagonunez/openqup/releases/latest/download/install.sh | sh
#   curl ... | sh -s -- --ref v0.2.0        (arguments go to `openqup install`)
#
# Downloads the openqup binary for this computer from the latest openqup release, checks it
# against the release's SHA256SUMS, puts it in ~/.local/bin (OPENQUP_BIN_DIR overrides) and runs
# `openqup install`, which builds and installs OpenQ.
set -eu

REPO="${OPENQUP_REPO:-iagonunez/openqup}"
BASE="${OPENQUP_DOWNLOAD:-https://github.com/$REPO/releases/latest/download}"
BIN_DIR="${OPENQUP_BIN_DIR:-$HOME/.local/bin}"

say() { printf 'openqup: %s\n' "$*" >&2; }
die() { say "$*"; exit 1; }

case "$(uname -s)" in
  Linux) os=unknown-linux-gnu ;;
  Darwin) os=apple-darwin ;;
  *) die "unsupported system $(uname -s); on Windows use install.ps1" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) die "unsupported processor $(uname -m)" ;;
esac
# A shell running under Rosetta reports x86_64 on Apple silicon: use the native build.
if [ "$os" = apple-darwin ] && [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ]; then
  arch=aarch64
fi
target="$arch-$os"

download() {
  if command -v curl >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget --https-only -q -O "$2" "$1"
  else
    die "curl or wget is needed"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading openqup for $target"
download "$BASE/openqup-$target" "$tmp/openqup"
download "$BASE/SHA256SUMS" "$tmp/SHA256SUMS"
want="$(awk -v f="openqup-$target" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")"
got="$(sha256 "$tmp/openqup")"
[ -n "$want" ] || die "openqup-$target isn't listed in SHA256SUMS"
[ "$want" = "$got" ] || die "checksum mismatch for openqup-$target (expected $want, got $got)"

if [ "$os" = apple-darwin ] && command -v codesign >/dev/null 2>&1; then
  if codesign --verify --strict "$tmp/openqup" 2>/dev/null; then
    say "signature: $(codesign -dv "$tmp/openqup" 2>&1 | sed -n 's/^Authority=//p' | head -n 1)"
  fi
fi

mkdir -p "$BIN_DIR"
mv "$tmp/openqup" "$BIN_DIR/openqup"
chmod 755 "$BIN_DIR/openqup"
say "installed $BIN_DIR/openqup"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "add $BIN_DIR to your PATH to run openqup later (openqup update, openqup uninstall)" ;;
esac

# Piped into sh, stdin is this script: answer openqup's questions from the terminal.
if [ -t 0 ] || ! (exec </dev/tty) 2>/dev/null; then
  exec "$BIN_DIR/openqup" install "$@"
else
  exec "$BIN_DIR/openqup" install "$@" </dev/tty
fi
