#!/bin/sh
# Install t9 from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/anupkalburgi/t9/main/install.sh | sh
#
# Environment:
#   T9_VERSION       release tag to install, e.g. v0.1.1 (default: the latest release)
#   T9_INSTALL_DIR   where to put the binary (default: ~/.local/bin)
#   T9_DOWNLOAD_URL  base URL of the release assets (for mirrors and testing)
set -eu

REPO="anupkalburgi/t9"
INSTALL_DIR="${T9_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 't9 install: %s\n' "$*"; }
fail() { printf 't9 install: error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "'$1' is required"; }

need uname
need tar
need mkdir
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q "$1" -O "$2"; }
else
  fail "curl or wget is required"
fi

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) os_part="apple-darwin" ;;
  Linux) os_part="unknown-linux-gnu" ;;
  *) fail "unsupported OS '$os' (macOS and Linux are supported; or build from source with cargo)" ;;
esac
case "$arch" in
  arm64 | aarch64) arch_part="aarch64" ;;
  x86_64 | amd64) arch_part="x86_64" ;;
  *) fail "unsupported CPU '$arch' (arm64 and x86_64 are supported)" ;;
esac
target="${arch_part}-${os_part}"
asset="t9-${target}.tar.gz"

if [ -n "${T9_DOWNLOAD_URL:-}" ]; then
  base="$T9_DOWNLOAD_URL"
elif [ -n "${T9_VERSION:-}" ]; then
  base="https://github.com/${REPO}/releases/download/${T9_VERSION}"
else
  base="https://github.com/${REPO}/releases/latest/download"
fi

tmp="$(mktemp -d 2>/dev/null || mktemp -d -t t9)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading ${asset} (${T9_VERSION:-latest})"
fetch "${base}/${asset}" "${tmp}/${asset}" || fail "could not download ${base}/${asset}"
fetch "${base}/SHA256SUMS" "${tmp}/SHA256SUMS" || fail "could not download ${base}/SHA256SUMS"

expected="$(grep " ${asset}\$" "${tmp}/SHA256SUMS" | cut -d' ' -f1)"
[ -n "$expected" ] || fail "no checksum for ${asset} in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${tmp}/${asset}" | cut -d' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "${tmp}/${asset}" | cut -d' ' -f1)"
else
  fail "sha256sum or shasum is required to verify the download"
fi
[ "$expected" = "$actual" ] || fail "checksum mismatch for ${asset} (expected ${expected}, got ${actual})"

tar -xzf "${tmp}/${asset}" -C "$tmp"
[ -f "${tmp}/t9" ] || fail "the archive does not contain a t9 binary"
mkdir -p "$INSTALL_DIR"
# Replace atomically, so a running t9 (e.g. an open TUI) keeps working.
cp "${tmp}/t9" "${INSTALL_DIR}/.t9.new"
chmod 755 "${INSTALL_DIR}/.t9.new"
mv -f "${INSTALL_DIR}/.t9.new" "${INSTALL_DIR}/t9"

say "installed $("${INSTALL_DIR}/t9" --version) to ${INSTALL_DIR}/t9"
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *) say "add ${INSTALL_DIR} to your PATH, e.g.: echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.zshrc" ;;
esac
cat <<EOF

Next:
  t9 install claude      # Claude Code plugin (prints two commands to run)
  t9 install agents-md   # AGENTS.md section for Codex and others
  cd your-project && t9 project init && t9 tui
EOF
