#!/usr/bin/env bash
set -euo pipefail

# Build test-only shared libraries on the native Intel runner;
# x264 is linked statically into each FFmpeg installation.
test "$(uname -m)" = x86_64
root="$GITHUB_WORKSPACE/target/ci-ffmpeg/x86_64"
sources="$RUNNER_TEMP/valle-ffmpeg-sources"
stamp="$(shasum -a 256 "$0" | cut -d ' ' -f 1)"
mkdir -p "$root" "$sources"

download() {
  local url="$1" archive="$2" checksum="$3"
  curl --fail --silent --show-error --location --retry 3 "$url" -o "$archive"
  printf '%s  %s\n' "$checksum" "$archive" | shasum -a 256 -c -
}

if [ ! -f "$root/.complete" ] || [ "$(cat "$root/.complete")" != "$stamp" ]; then
  sdk="$(xcrun --show-sdk-path)"
  flags='-mmacosx-version-min=15.0'
  jobs="${CARGO_BUILD_JOBS:-2}"
  x264=31e19f92f00c7003fa115047ce50978bc98c3a0d
  download "https://github.com/mirror/x264/archive/$x264.tar.gz" "$sources/x264.tar.gz" \
    d053c9d86988d6bc78237ca5205865c5ddf99c98ef4cd9927eec8f6d388f6dd9
  tar -xf "$sources/x264.tar.gz" -C "$sources"
  (
    cd "$sources/x264-$x264"
    CC="$(xcrun -f clang)" ./configure --prefix="$root/x264" \
      --sysroot="$sdk" \
      --extra-cflags="$flags" --extra-ldflags="$flags" \
      --enable-static --enable-pic --disable-cli --disable-asm --disable-opencl
    make -j "$jobs"
    make install
  )

  for version in 7.1.5 8.1.3 9.0.2; do
    case "$version" in
      7.1.5) checksum=de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f ;;
      8.1.3) checksum=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3 ;;
      9.0.2) checksum=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e ;;
    esac
    prefix="$root/${version%%.*}"
    download "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz" "$sources/ffmpeg-$version.tar.xz" "$checksum"
    tar -xf "$sources/ffmpeg-$version.tar.xz" -C "$sources"
    (
      cd "$sources/ffmpeg-$version"
      PKG_CONFIG_LIBDIR="$root/x264/lib/pkgconfig" ./configure \
        --prefix="$prefix" --cc="$(xcrun -f clang)" --sysroot="$sdk" \
        --extra-cflags="$flags" --extra-ldflags="$flags" --pkg-config=pkg-config \
        --disable-autodetect --disable-doc --disable-debug --disable-asm \
        --disable-static --enable-shared --enable-gpl --enable-libx264 --enable-zlib
      make -j "$jobs"
      make install
    )
  done
  printf '%s\n' "$stamp" > "$root/.complete"
fi

for major in 7 8 9; do
  prefix="$root/$major"
  lipo "$prefix/lib/libavcodec.dylib" -verify_arch x86_64
  "$prefix/bin/ffmpeg" -version > "$PACKAGE_LOGS/ffmpeg-cli-$major.txt"
  grep -E "^ffmpeg version ${major}[. ]" "$PACKAGE_LOGS/ffmpeg-cli-$major.txt"
  echo "VALLE_TEST_FFMPEG${major}_DIR=$prefix/lib" >> "$GITHUB_ENV"
done
echo "VALLE_FFMPEG_DIR=$root/9/lib" >> "$GITHUB_ENV"
echo "$root/9/bin" >> "$GITHUB_PATH"
