#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/install.sh"

if [[ ! -f "$script" ]]; then
  echo "install.sh does not exist" >&2
  exit 1
fi

bash -n "$script"

if ! grep -q "https://api.github.com/repos/kibuniverse/sparrow_agent/releases/latest" "$script"; then
  echo "install.sh must query the latest GitHub release" >&2
  exit 1
fi

if grep -q "releases/download/v0.0.2" "$script"; then
  echo "install.sh must not hard-code the v0.0.2 release asset URL" >&2
  exit 1
fi

if ! grep -q "sparrow_agent" "$script"; then
  echo "install.sh must install the sparrow_agent command" >&2
  exit 1
fi

if ! grep -q "spa" "$script"; then
  echo "install.sh must install the spa command alias" >&2
  exit 1
fi

case "$(uname -s)" in
  Darwin)
    case "$(uname -m)" in
      arm64 | aarch64) target="aarch64-apple-darwin" ;;
      x86_64 | amd64) target="x86_64-apple-darwin" ;;
      *) echo "unsupported test architecture: $(uname -m)" >&2; exit 1 ;;
    esac
    ;;
  Linux)
    case "$(uname -m)" in
      x86_64 | amd64) target="x86_64-unknown-linux-gnu" ;;
      *) echo "skipping functional install test on unsupported Linux architecture" >&2; exit 0 ;;
    esac
    ;;
  *)
    echo "skipping functional install test on unsupported OS: $(uname -s)" >&2
    exit 0
    ;;
esac

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

asset_name="sparrow_agent-$target.tar.xz"
payload_dir="$tmp_dir/payload/sparrow_agent-$target"
mock_bin="$tmp_dir/mock-bin"
bin_dir="$tmp_dir/global-bin"
archive="$tmp_dir/$asset_name"
checksum_file="$tmp_dir/$asset_name.sha256"
release_json="$tmp_dir/latest-release.json"

mkdir -p "$payload_dir" "$mock_bin" "$bin_dir"
printf '#!/usr/bin/env sh\nprintf "mock sparrow_agent\\n"\n' > "$payload_dir/sparrow_agent"
chmod 0755 "$payload_dir/sparrow_agent"
printf '# Sparrow Agent\n' > "$payload_dir/README.md"
tar -cJf "$archive" -C "$tmp_dir/payload" "sparrow_agent-$target"

if command -v sha256sum >/dev/null 2>&1; then
  archive_sha="$(sha256sum "$archive" | awk '{print $1}')"
else
  archive_sha="$(shasum -a 256 "$archive" | awk '{print $1}')"
fi
printf '%s *%s\n' "$archive_sha" "$asset_name" > "$checksum_file"

cat > "$release_json" <<JSON
{
  "tag_name": "v9.9.9",
  "assets": [
    { "browser_download_url": "https://example.invalid/$asset_name" },
    { "browser_download_url": "https://example.invalid/$asset_name.sha256" }
  ]
}
JSON

cat > "$mock_bin/curl" <<'MOCK_CURL'
#!/usr/bin/env bash
set -euo pipefail

url=""
output=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o)
      output="$2"
      shift 2
      ;;
    -H)
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

if [[ -z "$output" ]]; then
  echo "mock curl expected -o <file>" >&2
  exit 1
fi

case "$url" in
  https://api.github.com/repos/kibuniverse/sparrow_agent/releases/latest)
    cp "$SPARROW_TEST_RELEASE_JSON" "$output"
    ;;
  *.tar.xz)
    cp "$SPARROW_TEST_ARCHIVE" "$output"
    ;;
  *.tar.xz.sha256)
    cp "$SPARROW_TEST_CHECKSUM" "$output"
    ;;
  *)
    echo "unexpected mock curl URL: $url" >&2
    exit 1
    ;;
esac
MOCK_CURL
chmod 0755 "$mock_bin/curl"

PATH="$mock_bin:$PATH" \
SPARROW_TEST_RELEASE_JSON="$release_json" \
SPARROW_TEST_ARCHIVE="$archive" \
SPARROW_TEST_CHECKSUM="$checksum_file" \
SPARROW_AGENT_HOME="$tmp_dir/home" \
SPARROW_AGENT_BIN_DIR="$bin_dir" \
bash "$script" > "$tmp_dir/install-output.txt"

if ! grep -q "Download URL: https://api.github.com/repos/kibuniverse/sparrow_agent/releases/latest" "$tmp_dir/install-output.txt"; then
  echo "install.sh must print the full latest release metadata download URL" >&2
  exit 1
fi

if ! grep -q "Download URL: https://example.invalid/$asset_name" "$tmp_dir/install-output.txt"; then
  echo "install.sh must print the full release artifact download URL" >&2
  exit 1
fi

if ! grep -q "Download URL: https://example.invalid/$asset_name.sha256" "$tmp_dir/install-output.txt"; then
  echo "install.sh must print the full release artifact checksum download URL" >&2
  exit 1
fi

[[ -L "$bin_dir/sparrow_agent" ]] || { echo "sparrow_agent link was not created" >&2; exit 1; }
[[ -L "$bin_dir/spa" ]] || { echo "spa link was not created" >&2; exit 1; }

if [[ "$("$bin_dir/sparrow_agent")" != "mock sparrow_agent" ]]; then
  echo "sparrow_agent link does not execute installed binary" >&2
  exit 1
fi

if [[ "$("$bin_dir/spa")" != "mock sparrow_agent" ]]; then
  echo "spa link does not execute installed binary" >&2
  exit 1
fi
