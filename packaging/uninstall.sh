#!/usr/bin/env bash
set -euo pipefail
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -f -- "$HOME/.local/bin/rdnmb" \
  "$data_dir/applications/org.niconi.rdnmb.desktop" \
  "$data_dir/icons/hicolor/scalable/apps/org.niconi.rdnmb.svg"
if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$data_dir/applications" || true
fi
printf 'Removed R岛 executable, desktop entry and icon.\n'
