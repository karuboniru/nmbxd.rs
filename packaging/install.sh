#!/usr/bin/env bash
set -euo pipefail
bundle_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bin_dir="$HOME/.local/bin"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
# Desktop Exec values require an extra escaping layer beyond double quotes.
escape_exec() {
  sed 's/\\/\\\\\\\\/g; s/"/\\\\"/g; s/`/\\\\`/g; s/\$/\\\\$/g'
}
case "$bin_dir$data_dir" in
  *$'\n'*|*$'\r'*) echo 'Install paths must not contain newlines' >&2; exit 1 ;;
esac
install -Dm755 "$bundle_dir/rdnmb" "$bin_dir/rdnmb"
install -Dm644 "$bundle_dir/org.niconi.rdnmb.svg" "$data_dir/icons/hicolor/scalable/apps/org.niconi.rdnmb.svg"
mkdir -p "$data_dir/applications"
exec_path=$(printf '%s' "$bin_dir/rdnmb" | escape_exec)
while IFS= read -r line; do
  case "$line" in
    Exec=*) printf 'Exec="%s"\n' "$exec_path" ;;
    TryExec=*) : ;;
    *) printf '%s\n' "$line" ;;
  esac
done < "$bundle_dir/org.niconi.rdnmb.desktop" > "$data_dir/applications/org.niconi.rdnmb.desktop"
if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$data_dir/applications" || true
fi
printf 'Installed R岛: %s/rdnmb\n' "$bin_dir"
