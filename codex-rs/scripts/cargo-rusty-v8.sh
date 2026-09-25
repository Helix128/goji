#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cargo_root="$repo_root/codex-rs"
cargo_args=("$@")

if ((${#cargo_args[@]} == 0)); then
    cargo_args=(build)
fi

target="${RUSTY_V8_TARGET:-}"
for ((index = 0; index < ${#cargo_args[@]}; index++)); do
    case "${cargo_args[index]}" in
        --target=*) target="${cargo_args[index]#--target=}" ;;
        --target)
            index=$((index + 1))
            target="${cargo_args[index]:-}"
            ;;
    esac
done

if [[ -z "$target" ]]; then
    target="$(rustc -vV | sed -n 's/^host: //p')"
fi

version="$(python3 "$repo_root/.github/scripts/rusty_v8_bazel.py" resolved-v8-crate-version)"
version_slug="${version//./_}"
profile="ptrcomp_sandbox_release"
release_tag="rusty-v8-v${version}"
base_url="https://github.com/openai/codex/releases/download/${release_tag}"
manifest="$repo_root/third_party/v8/rusty_v8_${version_slug}_release_manifests.sha256"
artifact_dir="$cargo_root/target/rusty_v8/$version/$target"

case "$target" in
    *-pc-windows-msvc)
        archive_name="rusty_v8_${profile}_${target}.lib.gz"
        ;;
    *-apple-darwin|*-unknown-linux-gnu|*-unknown-linux-musl)
        archive_name="librusty_v8_${profile}_${target}.a.gz"
        ;;
    *)
        printf 'No published sandbox V8 artifact for target %s.\n' "$target" >&2
        exit 1
        ;;
esac

binding_name="src_binding_${profile}_${target}.rs"
checksums_name="rusty_v8_${profile}_${target}.sha256"
checksums_path="$artifact_dir/$checksums_name"

if [[ ! -f "$manifest" ]]; then
    printf 'Missing trusted V8 checksum manifest: %s\n' "$manifest" >&2
    exit 1
fi

mkdir -p "$artifact_dir"
curl -fsSL --retry 3 --retry-delay 1 "$base_url/$checksums_name" -o "$checksums_path"

if command -v sha256sum >/dev/null 2>&1; then
    digest() { sha256sum "$1" | awk '{print $1}'; }
else
    digest() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

expected_manifest_digest="$(awk -v name="$checksums_name" '$2 == name { print $1; exit }' "$manifest")"
actual_manifest_digest="$(digest "$checksums_path")"
if [[ -z "$expected_manifest_digest" || "$actual_manifest_digest" != "$expected_manifest_digest" ]]; then
    printf 'V8 checksum manifest does not match the trusted repository manifest.\n' >&2
    exit 1
fi

for artifact_name in "$archive_name" "$binding_name"; do
    artifact_path="$artifact_dir/$artifact_name"
    if [[ ! -f "$artifact_path" ]]; then
        curl -fsSL --retry 3 --retry-delay 1 "$base_url/$artifact_name" -o "$artifact_path"
    fi
done

(
    cd "$artifact_dir"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum --check --strict "$checksums_name"
    else
        shasum -a 256 --check "$checksums_name"
    fi
)

export RUSTY_V8_ARCHIVE="$artifact_dir/$archive_name"
export RUSTY_V8_SRC_BINDING_PATH="$artifact_dir/$binding_name"
cd "$cargo_root"
exec cargo "${cargo_args[@]}"
