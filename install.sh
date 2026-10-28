#!/bin/sh
# Install the `psl` command on this computer.
#
#   curl -fsSL https://raw.githubusercontent.com/wesleyjunkins/PSL/main/install.sh | sh
#                                 download the latest release for this machine
#   ./install.sh                  inside a clone with cargo: build from source; otherwise download
#   ./install.sh --prebuilt       download the release even inside a clone
#   ./install.sh --source         build this checkout (needs Rust from https://rustup.rs)
#   ./install.sh --version v0.1.0 download that release instead of the latest
#   ./install.sh --prefix DIR     install into DIR (default ~/.local/bin, or /usr/local/bin)
#   ./install.sh --uninstall      remove the installed binary
#
# Environment: PSL_REPO (owner/name on GitHub), PSL_VERSION (a tag, or "latest"),
# PSL_DOWNLOAD_BASE (a mirror of the GitHub releases directory). Prebuilt
# binaries exist for macOS (Apple Silicon, Intel) and Linux (x86_64, arm64).
set -e

REPO="${PSL_REPO:-wesleyjunkins/PSL}"
VERSION="${PSL_VERSION:-latest}"
BASE="${PSL_DOWNLOAD_BASE:-https://github.com/$REPO/releases}"
PREFIX=""
MODE=""
UNINSTALL=0

usage() {
  cat <<USAGE
usage: install.sh [--prebuilt | --source] [--version TAG] [--prefix DIR] [--uninstall]

  --prebuilt      download the release for this machine from GitHub (default outside a clone)
  --source        build the checkout next to this script with cargo (default inside a clone)
  --version TAG   download that release instead of the latest, e.g. v0.1.0
  --prefix DIR    install into DIR (default ~/.local/bin, or /usr/local/bin)
  --uninstall     remove the installed binary; workspaces are untouched
USAGE
}

need_arg() {
  if [ $# -lt 2 ] || [ -z "$2" ]; then echo "install.sh: $1 needs a value" >&2; exit 2; fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) need_arg "$@"; PREFIX="$2"; shift 2 ;;
    --version) need_arg "$@"; VERSION="$2"; shift 2 ;;
    --prebuilt) MODE=prebuilt; shift ;;
    --source) MODE=source; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "install.sh: unknown option $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ -z "$PREFIX" ]; then
  if [ -d "$HOME/.local/bin" ] && [ -w "$HOME/.local/bin" ]; then PREFIX="$HOME/.local/bin"
  elif [ -w /usr/local/bin ]; then PREFIX=/usr/local/bin
  else PREFIX="$HOME/.local/bin"
  fi
fi

if [ "$UNINSTALL" = 1 ]; then
  if [ -f "$PREFIX/psl" ]; then rm -f "$PREFIX/psl"; echo "removed $PREFIX/psl"; else echo "nothing installed at $PREFIX/psl"; fi
  exit 0
fi

# When piped from curl, $0 is the shell and HERE is the current directory.
HERE=$(cd "$(dirname "$0")" 2>/dev/null && pwd)
if [ -z "$MODE" ]; then
  if [ -f "$HERE/core/Cargo.toml" ]; then
    if command -v cargo >/dev/null 2>&1; then MODE=source
    else echo "no cargo on this machine; downloading a prebuilt release instead of building this checkout"; MODE=prebuilt
    fi
  else MODE=prebuilt
  fi
fi

fetch() { # fetch URL DEST
  if command -v curl >/dev/null 2>&1; then curl -fsSL -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then wget -q -O "$2" "$1"
  else echo "install.sh: downloading needs curl or wget" >&2; exit 1
  fi
}

install_prebuilt() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$arch" in
    arm64|aarch64) arch=aarch64 ;;
    x86_64|amd64) arch=x86_64 ;;
    *) echo "install.sh: no prebuilt psl for the $arch processor; build from source (see --help)" >&2; exit 1 ;;
  esac
  case "$os" in
    Darwin) target="$arch-apple-darwin" ;;
    Linux) target="$arch-unknown-linux-gnu" ;;
    *) echo "install.sh: no prebuilt psl for $os yet (macOS and Linux only)" >&2; exit 1 ;;
  esac
  asset="psl-$target.tar.gz"
  if [ "$VERSION" = latest ]; then dir="$BASE/latest/download"; else dir="$BASE/download/$VERSION"; fi

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT

  echo "downloading $dir/$asset ..."
  if ! fetch "$dir/$asset" "$tmp/$asset"; then
    echo "install.sh: download failed; see https://github.com/$REPO/releases for the published versions" >&2
    exit 1
  fi

  if fetch "$dir/SHA256SUMS" "$tmp/SHA256SUMS" 2>/dev/null; then
    expected=$(grep "[ *]$asset\$" "$tmp/SHA256SUMS" | awk '{print $1}')
    if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$tmp/$asset" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then actual=$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')
    else actual=""
    fi
    if [ -n "$expected" ] && [ -n "$actual" ]; then
      if [ "$expected" != "$actual" ]; then
        echo "install.sh: checksum mismatch for $asset (expected $expected, got $actual)" >&2
        exit 1
      fi
      echo "checksum ok"
    else
      echo "install.sh: warning: could not verify the checksum of $asset" >&2
    fi
  else
    echo "install.sh: warning: no SHA256SUMS published alongside $asset; skipping verification" >&2
  fi

  tar -xzf "$tmp/$asset" -C "$tmp" psl
  mkdir -p "$PREFIX"
  cp "$tmp/psl" "$PREFIX/psl"
  chmod 755 "$PREFIX/psl"
  if [ "$os" = Darwin ]; then xattr -d com.apple.quarantine "$PREFIX/psl" 2>/dev/null || true; fi
}

install_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "install.sh: building psl from source needs Rust (cargo)." >&2
    echo "  Install it from https://rustup.rs (one command), open a new terminal and rerun ./install.sh" >&2
    exit 1
  fi
  if [ ! -f "$HERE/core/Cargo.toml" ]; then
    echo "install.sh: --source must run from a clone of the repository (no core/Cargo.toml next to this script)" >&2
    exit 1
  fi
  echo "building psl (release build, first time takes about a minute) ..."
  cargo build --release --quiet --manifest-path "$HERE/core/Cargo.toml"
  mkdir -p "$PREFIX"
  cp "$HERE/core/target/release/psl" "$PREFIX/psl"
  chmod 755 "$PREFIX/psl"
}

case "$MODE" in
  prebuilt) install_prebuilt ;;
  source) install_source ;;
esac
echo "installed $PREFIX/psl  ($("$PREFIX/psl" version))"

case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *)
    echo
    echo "$PREFIX is not on your PATH. Add this line to ~/.zshrc (or ~/.bashrc), then open a new terminal:"
    echo "  export PATH=\"$PREFIX:\$PATH\""
    ;;
esac
echo
echo "next: cd into any folder, then   psl init   and   psl run"
