#!/usr/bin/env bash
# Build a native macOS binary and package only public distribution files.
set -euo pipefail
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_dir"
skip_build=false
target=${CARGO_BUILD_TARGET:-}
while (($#)); do
  case "$1" in
    --skip-build) skip_build=true; shift ;;
    --target)
      [[ $# -ge 2 ]] || { echo '--target requires a Rust target' >&2; exit 2; }
      target=$2
      shift 2
      ;;
    *) echo 'Usage: scripts/package-macos.sh [--skip-build] [--target TARGET]' >&2; exit 2 ;;
  esac
done
[[ $(uname -s) == Darwin ]] || { echo 'Run this script on macOS with Xcode installed' >&2; exit 1; }
case $(uname -m) in
  arm64) native_target=aarch64-apple-darwin; arch=aarch64; macho_arch=arm64 ;;
  x86_64) native_target=x86_64-apple-darwin; arch=x86_64; macho_arch=x86_64 ;;
  *) echo 'Unsupported macOS architecture' >&2; exit 1 ;;
esac
target=${target:-$native_target}
[[ $target == "$native_target" ]] || { echo "Use the native Rust target: $native_target" >&2; exit 1; }
if [[ $skip_build == false ]]; then
  cargo build --release --locked -p rdnmb --target "$target"
fi
version=$(sed -n '/^\[package\]/,/^\[/s/^version = "\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
[[ $version =~ ^[A-Za-z0-9._+-]+$ ]] || { echo 'Invalid package version' >&2; exit 1; }
name="rdnmb-${version}-macos-${arch}"
binary="${CARGO_TARGET_DIR:-target}/$target/release/rdnmb"
[[ -f $binary && -x $binary ]] || { echo "Release binary missing: $binary" >&2; exit 1; }
# Check the actual executable before labelling the archive with its architecture.
lipo -verify_arch "$macho_arch" "$binary"
mkdir -p dist
stage=$(mktemp -d "$project_dir/dist/.package.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
bundle="$stage/$name"
mkdir -p "$bundle/docs" "$bundle/licenses"
install -m755 "$binary" "$bundle/rdnmb"
install -m644 README.md .env.example "$bundle/"
install -m644 docs/*.md docs/screenshot.png "$bundle/docs/"
install -m644 assets/fonts/OFL.txt "$bundle/licenses/NotoColorEmoji-OFL.txt"
install -m644 vendor/cosmic-text/unicode/LICENSE.txt "$bundle/licenses/Unicode.txt"
install -m644 vendor/cosmic-text/LICENSE-MIT "$bundle/licenses/cosmic-text-MIT.txt"
install -m644 vendor/cosmic-text/LICENSE-APACHE "$bundle/licenses/cosmic-text-APACHE.txt"
install -m644 vendor/gpui-component/LICENSE-APACHE "$bundle/licenses/gpui-component.txt"
if [[ -f LICENSE ]]; then install -m644 LICENSE "$bundle/LICENSE"; fi
otool -L "$binary" > "$bundle/DEPENDENCIES.txt"
{
  sw_vers
  printf 'Rust target: %s\n' "$target"
  printf 'MACOSX_DEPLOYMENT_TARGET: %s\n' "${MACOSX_DEPLOYMENT_TARGET:-toolchain default}"
  xcodebuild -version
} > "$bundle/BUILD-OS.txt"
# BSD tar is available on macOS. Omit Finder metadata and gzip timestamps;
# this does not claim reproducible builds or fixed archive file timestamps.
COPYFILE_DISABLE=1 tar -C "$stage" -cf - "$name" | gzip -n > "dist/$name.tar.gz"
(cd dist && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
rm -rf -- "dist/$name"
mv "$bundle" "dist/$name"
printf 'Created dist/%s.tar.gz and dist/%s.tar.gz.sha256\n' "$name" "$name"
