#!/usr/bin/env bash
set -euo pipefail

root="${1:?Usage: verify-release.sh ARTIFACTS TAG}"
tag="${2:?Usage: verify-release.sh ARTIFACTS TAG}"

archives=()
count=0
while IFS= read -r -d '' archive; do
  archives+=("$archive")
  count=$((count + 1))
done < <(find "$root" -type f \( -name '*.tar.gz' -o -name '*.zip' \) -print0)

if [ "$count" -ne 6 ]; then
  printf 'Expected six %s platform archives; got %s\n' "$tag" "$count" >&2
  exit 1
fi

for platform in linux-{arm64,x86_64} darwin-{arm64,x86_64} windows-{arm64,x86_64}; do
  extension=tar.gz
  if [[ "$platform" == windows-* ]]; then
    extension=zip
  fi
  name="valle-$tag-$platform.$extension"
  matches=0
  for archive in "${archives[@]}"; do
    if [ "${archive##*/}" != "$name" ]; then
      continue
    fi
    matches=$((matches + 1))
    checksum="$(cat "$archive.sha256")"
    read -r digest target extra <<< "$checksum"
    if [[ ! "$digest" =~ ^[[:xdigit:]]{64}$ || "${target#\*}" != "$name" ||
      -n "$extra" || "$checksum" == *$'\n'* ]]; then
      printf 'Invalid checksum in %s.sha256\n' "$archive" >&2
      exit 1
    fi
    (cd "$(dirname "$archive")" && sha256sum -c "$name.sha256")
  done
  if [ "$matches" -ne 1 ]; then
    printf 'Expected one %s; got %s\n' "$name" "$matches" >&2
    exit 1
  fi
done
