#!/usr/bin/env bash
set -euo pipefail
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -f -- "$HOME/.local/bin/nmbxd" \
  "$data_dir/applications/io.github.nmbxd.Dawn.desktop" \
  "$data_dir/icons/hicolor/scalable/apps/io.github.nmbxd.Dawn.svg"
if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$data_dir/applications" || true
fi
printf 'Removed X岛黎明版 executable, desktop entry and icon.\n'
