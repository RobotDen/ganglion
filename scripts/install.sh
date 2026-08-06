#!/usr/bin/env bash
# Ganglion (`gang`) installer.
#
#   curl -fsSL https://raw.githubusercontent.com/RobotDen/ganglion/main/scripts/install.sh | sh
#
# Downloads the prebuilt `gang` binary for this platform from the latest
# GitHub release, verifies its SHA-256 against the published SHA256SUMS, and
# installs it. No Rust toolchain required.
#
# Environment overrides:
#   GANG_VERSION   install a specific tag (e.g. v2.1.0). Default: latest release.
#   GANG_BIN_DIR   install directory. Default: ~/.local/bin (falls back to a
#                  sudo install into /usr/local/bin if that dir isn't writable
#                  and is not on PATH).
set -eu

REPO="RobotDen/ganglion"
BIN="gang"

say()  { printf '\033[1;36m==>\033[0m %s\n' "$1"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$1" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "required tool not found: $1"; }
need curl
need tar

# --- Detect platform -> release target triple -------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  os_part="unknown-linux-gnu" ;;
  Darwin) os_part="apple-darwin" ;;
  *) die "unsupported OS: $os (build from source: cargo install gang)" ;;
esac
case "$arch" in
  x86_64|amd64)  arch_part="x86_64" ;;
  aarch64|arm64) arch_part="aarch64" ;;
  *) die "unsupported architecture: $arch (build from source: cargo install gang)" ;;
esac
target="${arch_part}-${os_part}"

# --- Resolve version ---------------------------------------------------------
version="${GANG_VERSION:-}"
if [ -z "$version" ]; then
  say "Resolving latest release..."
  version="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 \
    | sed 's/.*"\([^"]*\)"$/\1/')"
  [ -n "$version" ] || die "could not determine the latest release tag; set GANG_VERSION=vX.Y.Z"
fi
ver_no_v="${version#v}"
say "Installing ${BIN} ${version} (${target})"

tarball="${BIN}-v${ver_no_v}-${target}.tar.gz"
base="https://github.com/${REPO}/releases/download/${version}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# --- Download tarball + checksums --------------------------------------------
say "Downloading ${tarball}"
curl -fSL --proto '=https' --tlsv1.2 "${base}/${tarball}" -o "${tmp}/${tarball}" \
  || die "download failed — does a prebuilt binary exist for ${target} in ${version}? (build from source: cargo install gang)"
curl -fSL --proto '=https' --tlsv1.2 "${base}/SHA256SUMS" -o "${tmp}/SHA256SUMS" \
  || die "could not fetch SHA256SUMS for ${version}"

# --- Verify checksum ---------------------------------------------------------
say "Verifying SHA-256"
expected="$(grep " ${tarball}\$" "${tmp}/SHA256SUMS" | awk '{print $1}')"
[ -n "$expected" ] || die "no checksum for ${tarball} in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${tmp}/${tarball}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "${tmp}/${tarball}" | awk '{print $1}')"
else
  die "need sha256sum or shasum to verify the download"
fi
[ "$expected" = "$actual" ] || die "checksum mismatch for ${tarball} (expected ${expected}, got ${actual})"

# --- Unpack + install --------------------------------------------------------
tar -C "$tmp" -xzf "${tmp}/${tarball}"
[ -f "${tmp}/${BIN}" ] || die "archive did not contain the ${BIN} binary"
chmod +x "${tmp}/${BIN}"

bindir="${GANG_BIN_DIR:-$HOME/.local/bin}"
if mkdir -p "$bindir" 2>/dev/null && [ -w "$bindir" ]; then
  mv "${tmp}/${BIN}" "${bindir}/${BIN}"
  say "Installed to ${bindir}/${BIN}"
  case ":${PATH}:" in
    *":${bindir}:"*) : ;;
    *) warn "${bindir} is not on your PATH — add: export PATH=\"${bindir}:\$PATH\"" ;;
  esac
else
  say "Installing to /usr/local/bin (requires sudo)"
  sudo mv "${tmp}/${BIN}" "/usr/local/bin/${BIN}"
  say "Installed to /usr/local/bin/${BIN}"
fi

say "Done. Run: ${BIN} --version"
