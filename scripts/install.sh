#!/bin/sh
# Installs Valle on macOS and Linux:
#
#   curl -fsSL https://openvalle.ai/install.sh | sh
#
# It downloads the latest release's executable for this machine, checks it against the
# release's SHA256SUMS, and puts it in ~/.valle/bin. VALLE_VERSION installs another release
# (v0.1.0), VALLE_INSTALL another folder than ~/.valle.
set -eu

repository="openvalle/valle"
install="${VALLE_INSTALL:-$HOME/.valle}"
bin="$install/bin"

if [ -t 1 ]; then
  bold=$(printf '\033[1m') dim=$(printf '\033[2m') red=$(printf '\033[31m') green=$(printf '\033[32m') plain=$(printf '\033[0m')
else
  bold="" dim="" red="" green="" plain=""
fi

fail() {
  printf '%serror%s: %s\n' "$red" "$plain" "$1" >&2
  exit 1
}

need() {
  command -v "$1" > /dev/null 2>&1 || fail "$1 is needed to install Valle"
}

need curl
need tar
need uname

case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  *) fail "this script installs Valle on macOS and Linux; on Windows run: irm https://openvalle.ai/install.ps1 | iex" ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) fail "Valle has no package for $(uname -m)" ;;
esac

# A shell running under Rosetta says x86_64 on an Apple silicon Mac.
if [ "$os" = macos ] && [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2> /dev/null || echo 0)" = 1 ]; then
  arch=arm64
fi

if [ "$os" = linux ]; then
  # The packages are built for glibc, 2.28 or later.
  if ldd --version 2>&1 | grep -qi musl; then
    fail "Valle's Linux packages need glibc; this system uses musl"
  fi
  glibc=$(getconf GNU_LIBC_VERSION 2> /dev/null | awk '{print $2}')
  if [ -n "$glibc" ]; then
    major=${glibc%%.*}
    minor=${glibc#*.}
    minor=${minor%%.*}
    if [ "$major" -lt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -lt 28 ]; }; then
      fail "Valle needs glibc 2.28 or later; this system has $glibc"
    fi
  fi
fi

version="${VALLE_VERSION:-}"
if [ -z "$version" ]; then
  # The latest release, from where its page redirects to: no API, so no rate limit.
  latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repository/releases/latest") ||
    fail "could not reach github.com to find the latest release"
  version=${latest##*/}
fi
case "$version" in
  v[0-9]*) ;;
  [0-9]*) version="v$version" ;;
  *) fail "could not tell the latest release (got \"$version\"); set VALLE_VERSION, such as v0.1.0" ;;
esac

archive="valle-$version-$os-$arch.tar.gz"
url="https://github.com/$repository/releases/download/$version"

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT INT TERM

printf '%sDownloading%s Valle %s for %s %s\n' "$bold" "$plain" "$version" "$os" "$arch"
curl -fL --progress-bar -o "$temporary/$archive" "$url/$archive" ||
  fail "could not download $url/$archive"
curl -fsSL -o "$temporary/SHA256SUMS" "$url/SHA256SUMS" ||
  fail "could not download $url/SHA256SUMS"

expected=$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' "$temporary/SHA256SUMS")
[ -n "$expected" ] || fail "SHA256SUMS of $version does not list $archive"
if command -v sha256sum > /dev/null 2>&1; then
  actual=$(sha256sum "$temporary/$archive" | awk '{print $1}')
elif command -v shasum > /dev/null 2>&1; then
  actual=$(shasum -a 256 "$temporary/$archive" | awk '{print $1}')
else
  fail "sha256sum or shasum is needed to check the download"
fi
[ "$actual" = "$expected" ] || fail "$archive does not match its checksum: expected $expected, got $actual"

tar -xzf "$temporary/$archive" -C "$temporary" || fail "could not unpack $archive"
mkdir -p "$bin" || fail "could not create $bin"
# Over a running valle: a rename replaces the file without touching the one in use.
chmod +x "$temporary/valle"
mv -f "$temporary/valle" "$bin/valle.new" || fail "could not write $bin/valle.new"
mv -f "$bin/valle.new" "$bin/valle" || fail "could not write $bin/valle"

installed=$("$bin/valle" --version 2> /dev/null) || fail "$bin/valle does not run on this machine"

# A path under the home folder, shown with a tilde: text for a person, not a path to open.
# shellcheck disable=SC2088
tilde() {
  case "$1" in
    "$HOME"/*) printf '~/%s' "${1#"$HOME"/}" ;;
    *) printf '%s' "$1" ;;
  esac
}

printf '%s%s%s was installed to %s%s%s\n' "$green" "$installed" "$plain" "$bold" "$(tilde "$bin/valle")" "$plain"

# On PATH already: nothing more to do.
case ":$PATH:" in
  *":$bin:"*)
    printf '\nRun %svalle docs start%s to begin.\n' "$bold" "$plain"
    exit 0
    ;;
esac

# Else the shell's startup file gains the folder, once.
case "$bin" in
  "$HOME"/*) shown="\$HOME/${bin#"$HOME"/}" ;;
  *) shown="$bin" ;;
esac
line="export PATH=\"$shown:\$PATH\""
config=""
# The login shell, where there is one to name: a container or a CI job may set no SHELL.
shell="${SHELL:-}"
case "${shell##*/}" in
  zsh) config="${ZDOTDIR:-$HOME}/.zshrc" ;;
  bash)
    if [ "$os" = macos ]; then config="$HOME/.bash_profile"; else config="$HOME/.bashrc"; fi
    ;;
  fish)
    config="${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish"
    line="fish_add_path \"$shown\""
    ;;
esac

if [ -n "$config" ] && [ "${VALLE_NO_MODIFY_PATH:-}" != 1 ]; then
  mkdir -p "$(dirname "$config")"
  if ! grep -qsF "$shown" "$config"; then
    printf '\n# Valle\n%s\n' "$line" >> "$config"
    printf 'Added %s to PATH in %s\n' "$(tilde "$bin")" "$(tilde "$config")"
  fi
  printf '\nOpen a new terminal, or run:\n\n  %s%s%s\n\nThen %svalle docs start%s to begin.\n' "$bold" "$line" "$plain" "$bold" "$plain"
else
  printf '\nAdd Valle to your PATH:\n\n  %s%s%s\n\nThen %svalle docs start%s to begin.\n' "$bold" "$line" "$plain" "$bold" "$plain"
fi

if ! command -v ffmpeg > /dev/null 2>&1; then
  printf '\n%sVideo needs FFmpeg 7, 8 or 9: https://github.com/%s/blob/main/docs/install.md#ffmpeg%s\n' "$dim" "$repository" "$plain"
fi
