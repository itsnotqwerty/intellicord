#!/usr/bin/env bash
set -euo pipefail

data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/intellicord"
desktop="$data_dir/applications/discord.desktop"
if [[ -f "$desktop" ]] && grep -q '^X-Intellicord-Managed=true$' "$desktop"; then
  if [[ -f "$state_dir/discord.desktop.previous" ]]; then
    mv -- "$state_dir/discord.desktop.previous" "$desktop"
  else
    rm -- "$desktop"
  fi
fi
rm -f -- "$HOME/.local/bin/intellicord"
update-desktop-database "$data_dir/applications"
if command -v kbuildsycoca6 >/dev/null; then kbuildsycoca6 --noincremental; fi
printf 'Removed the user-local Intellicord installation. Running processes were not changed.\n'