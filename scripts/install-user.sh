#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/intellicord"
binary="$HOME/.local/bin/intellicord"
desktop="$data_dir/applications/discord.desktop"

for tool in npm cargo install desktop-file-install desktop-file-validate update-desktop-database; do
  command -v "$tool" >/dev/null || { printf 'Required tool unavailable: %s\n' "$tool" >&2; exit 1; }
done
cd "$project_dir"
npm ci
npm run build
cargo build --release --locked
install -d "$HOME/.local/bin" "$data_dir/applications"
install -d -m 700 "$state_dir"
if [[ -f "$desktop" ]] && ! grep -q '^X-Intellicord-Managed=true$' "$desktop"; then
  if [[ -e "$state_dir/discord.desktop.previous" ]]; then
    printf 'A previous launcher backup already exists; inspect it before replacing another custom launcher.\n' >&2
    exit 1
  fi
  cp -p -- "$desktop" "$state_dir/discord.desktop.previous"
fi
temporary_binary="$(mktemp "$HOME/.local/bin/.intellicord.XXXXXXXX")"
trap 'rm -f -- "$temporary_binary"' EXIT
install -m 755 target/release/intellicord "$temporary_binary"
mv -f -- "$temporary_binary" "$binary"
desktop-file-install --dir="$data_dir/applications" --set-key=Exec --set-value="\"$binary\" launch %u" packaging/discord.desktop
desktop-file-validate "$desktop"
update-desktop-database "$data_dir/applications"
if command -v kbuildsycoca6 >/dev/null; then kbuildsycoca6 --noincremental; fi
printf 'Installed %s\nDiscord launcher now attaches Intellicord on future launches.\nNo running Discord process was changed.\n' "$binary"