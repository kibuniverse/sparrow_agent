#!/usr/bin/env bash
set -euo pipefail

REPO="kibuniverse/sparrow_agent"
APP_NAME="sparrow_agent"
LATEST_RELEASE_API="https://api.github.com/repos/kibuniverse/sparrow_agent/releases/latest"

usage() {
  cat <<'USAGE'
Install the latest Sparrow Agent release.

Usage:
  ./install.sh

Environment:
  SPARROW_AGENT_HOME         Base directory for versioned installs.
                             Default: $HOME/.sparrow_agent
  SPARROW_AGENT_INSTALL_DIR  Exact directory to place the downloaded binary.
                             Default: $SPARROW_AGENT_HOME/releases/<tag>/<target>
  SPARROW_AGENT_BIN_DIR      Directory where global command links are created.
                             Default: $CARGO_HOME/bin, $HOME/.cargo/bin, or $HOME/.local/bin
  SPARROW_AGENT_FORCE=1      Replace existing non-symlink files in SPARROW_AGENT_BIN_DIR.
  GITHUB_TOKEN               Optional token for GitHub API/download rate limits.

Commands installed:
  sparrow_agent
  spa
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

default_bin_dir() {
  if [[ -n "${CARGO_HOME:-}" ]]; then
    printf '%s/bin\n' "$CARGO_HOME"
  elif [[ -d "$HOME/.cargo/bin" ]]; then
    printf '%s/.cargo/bin\n' "$HOME"
  else
    printf '%s/.local/bin\n' "$HOME"
  fi
}

detect_target() {
  local os
  local arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$arch" in
    x86_64 | amd64)
      arch="x86_64"
      ;;
    arm64 | aarch64)
      arch="aarch64"
      ;;
    *)
      fail "unsupported CPU architecture: $arch"
      ;;
  esac

  case "$os" in
    Darwin)
      printf '%s-apple-darwin\n' "$arch"
      ;;
    Linux)
      if [[ "$arch" != "x86_64" ]]; then
        fail "Linux release artifacts are currently only published for x86_64"
      fi
      printf 'x86_64-unknown-linux-gnu\n'
      ;;
    *)
      fail "unsupported operating system: $os. Use the PowerShell installer on Windows."
      ;;
  esac
}

curl_args=(-fsSL)
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  curl_args+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
fi
curl_args+=(-H "Accept: application/vnd.github+json")

download() {
  local url="$1"
  local output="$2"
  log "Download URL: $url"
  curl "${curl_args[@]}" "$url" -o "$output"
}

sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    fail "required command not found: sha256sum or shasum"
  fi
}

extract_json_string() {
  local key="$1"
  local file="$2"
  sed -n "s/.*\"$key\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" "$file" | head -n 1
}

find_asset_url() {
  local asset_name="$1"
  local file="$2"
  sed -n "s|.*\"browser_download_url\"[[:space:]]*:[[:space:]]*\"\\([^\"]*/${asset_name}\\)\".*|\\1|p" "$file" | head -n 1
}

ensure_link_target_is_replaceable() {
  local path="$1"
  if [[ -e "$path" && ! -L "$path" && "${SPARROW_AGENT_FORCE:-}" != "1" ]]; then
    fail "$path already exists and is not a symlink. Set SPARROW_AGENT_FORCE=1 to replace it."
  fi
}

warn_if_not_on_path() {
  local dir="$1"
  case ":${PATH}:" in
    *":${dir}:"*) ;;
    *)
      log "warning: $dir is not on PATH."
      log "Add this to your shell profile:"
      log "  export PATH=\"$dir:\$PATH\""
      ;;
  esac
}

need_cmd curl
need_cmd tar
need_cmd awk
need_cmd sed
need_cmd uname
need_cmd mktemp

target="$(detect_target)"
asset_name="${APP_NAME}-${target}.tar.xz"
checksum_name="${asset_name}.sha256"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

release_json="$tmp_dir/latest-release.json"
archive="$tmp_dir/$asset_name"
checksum_file="$tmp_dir/$checksum_name"
extract_dir="$tmp_dir/extract"

log "Fetching latest release metadata from GitHub..."
download "$LATEST_RELEASE_API" "$release_json"

tag="$(extract_json_string "tag_name" "$release_json")"
[[ -n "$tag" ]] || fail "could not determine latest release tag from GitHub API"

asset_url="$(find_asset_url "$asset_name" "$release_json")"
checksum_url="$(find_asset_url "$checksum_name" "$release_json")"

if [[ -z "$asset_url" ]]; then
  asset_url="https://github.com/${REPO}/releases/download/${tag}/${asset_name}"
fi
if [[ -z "$checksum_url" ]]; then
  checksum_url="${asset_url}.sha256"
fi

home_dir="${SPARROW_AGENT_HOME:-$HOME/.sparrow_agent}"
install_dir="${SPARROW_AGENT_INSTALL_DIR:-$home_dir/releases/$tag/$target}"
bin_dir="${SPARROW_AGENT_BIN_DIR:-$(default_bin_dir)}"

log "Installing $APP_NAME $tag for $target"
log "Downloading $asset_name..."
download "$asset_url" "$archive"

log "Downloading checksum..."
download "$checksum_url" "$checksum_file"

expected_sha="$(awk '{print $1}' "$checksum_file" | head -n 1)"
actual_sha="$(sha256_file "$archive")"
[[ -n "$expected_sha" ]] || fail "checksum file did not contain a SHA-256 value"
if [[ "$expected_sha" != "$actual_sha" ]]; then
  fail "checksum mismatch for $asset_name"
fi

mkdir -p "$extract_dir" "$install_dir" "$bin_dir"
tar -xf "$archive" -C "$extract_dir"

binary="$extract_dir/${APP_NAME}-${target}/${APP_NAME}"
if [[ ! -f "$binary" ]]; then
  binary="$(find "$extract_dir" -type f -name "$APP_NAME" -perm -111 | head -n 1)"
fi
[[ -n "${binary:-}" && -f "$binary" ]] || fail "could not find $APP_NAME in $asset_name"

installed_binary="$install_dir/$APP_NAME"
cp "$binary" "$installed_binary"
chmod 0755 "$installed_binary"

ensure_link_target_is_replaceable "$bin_dir/$APP_NAME"
ensure_link_target_is_replaceable "$bin_dir/spa"

ln -sfn "$installed_binary" "$bin_dir/$APP_NAME"
ln -sfn "$installed_binary" "$bin_dir/spa"

log "Installed:"
log "  $bin_dir/$APP_NAME -> $installed_binary"
log "  $bin_dir/spa -> $installed_binary"
warn_if_not_on_path "$bin_dir"
