#!/usr/bin/env bash
# Build and package only explicitly listed, public distribution files.
set -euo pipefail
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_dir"
if [[ ${1:-} == --skip-build ]]; then
  shift
else
  cargo build --release --locked
fi
if (($#)); then
  echo 'Usage: scripts/package-linux.sh [--skip-build]' >&2
  exit 2
fi
version=$(sed -n '/^\[package\]/,/^\[/s/^version = "\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
[[ $version =~ ^[A-Za-z0-9._+-]+$ ]] || { echo 'Invalid package version' >&2; exit 1; }
arch=$(uname -m)
[[ $arch =~ ^[A-Za-z0-9_+-]+$ ]] || { echo "Invalid architecture" >&2; exit 1; }
name="nmbxd-${version}-linux-${arch}"
binary="${CARGO_TARGET_DIR:-target}/release/nmbxd"
[[ -f $binary && -x $binary ]] || { echo "Release binary missing: $binary" >&2; exit 1; }
# GNU tar and gzip make archive metadata/order deterministic. Build reproducibility
# itself still depends on the Rust toolchain, linker and host libraries.
epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || printf '0')}
[[ $epoch =~ ^[0-9]+$ ]] || { echo 'SOURCE_DATE_EPOCH must be an integer' >&2; exit 1; }
mkdir -p dist
stage=$(mktemp -d "$project_dir/dist/.package.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
bundle="$stage/$name"
mkdir -p "$bundle"
install -m755 "$binary" "$bundle/nmbxd"
install -m644 packaging/io.github.nmbxd.Dawn.desktop packaging/io.github.nmbxd.Dawn.svg README.md "$bundle/"
install -m755 packaging/install.sh packaging/uninstall.sh "$bundle/"
mkdir -p "$bundle/docs" "$bundle/licenses"
install -m644 assets/fonts/OFL.txt "$bundle/licenses/NotoColorEmoji-OFL.txt"
install -m644 vendor/cosmic-text/unicode/LICENSE.txt "$bundle/licenses/Unicode.txt"
install -m644 vendor/cosmic-text/LICENSE-MIT "$bundle/licenses/cosmic-text-MIT.txt"
install -m644 vendor/cosmic-text/LICENSE-APACHE "$bundle/licenses/cosmic-text-APACHE.txt"
install -m644 vendor/gpui-component/LICENSE-APACHE "$bundle/licenses/gpui-component.txt"
install -m644 docs/dark-mode.png docs/settings.png "$bundle/docs/"
if [[ -f LICENSE ]]; then install -m644 LICENSE "$bundle/LICENSE"; fi
# Never copy .env or recursively copy the project/build tree.
install -m644 .env.example "$bundle/.env.example"
LC_ALL=C ldd "$binary" > "$bundle/DEPENDENCIES.txt"
if grep -q 'not found' "$bundle/DEPENDENCIES.txt"; then
  echo 'Release binary has missing shared libraries:' >&2
  cat "$bundle/DEPENDENCIES.txt" >&2
  exit 1
fi
# Addresses printed by ldd vary under ASLR; omit them for stable bundle bytes.
sed -E -i 's/ \(0x[[:xdigit:]]+\)//g' "$bundle/DEPENDENCIES.txt"
cp /etc/os-release "$bundle/BUILD-OS.txt"
tar --sort=name --format=gnu --mtime="@$epoch" --owner=0 --group=0 --numeric-owner \
  -C "$stage" -cf - "$name" | gzip -n > "dist/$name.tar.gz"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
# Replace only the generated directory for this exact version and architecture.
rm -rf -- "dist/$name"
mv "$bundle" "dist/$name"
printf 'Created dist/%s.tar.gz and dist/%s.tar.gz.sha256\n' "$name" "$name"
