#!/bin/sh
# Install GittyHub for the current user: binary, desktop entry and icon.
set -eu
cd "$(dirname "$0")"
APP_ID=io.github.Kelvinlby.GittyHub
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

cargo install --locked --path crates/app
install -Dm644 "data/$APP_ID.desktop" "$DATA/applications/$APP_ID.desktop"
install -Dm644 "data/$APP_ID.svg" "$DATA/icons/hicolor/scalable/apps/$APP_ID.svg"
# Point Exec at the installed binary so it works without ~/.cargo/bin on the session PATH.
BIN="${CARGO_HOME:-$HOME/.cargo}/bin/gittyhub"
sed -i "s|^Exec=gittyhub|Exec=$BIN|" "$DATA/applications/$APP_ID.desktop"
# Remove the entry used by early builds.
rm -f "$DATA/applications/gittyhub.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database -q "$DATA/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qtf "$DATA/icons/hicolor" || true
echo "Installed $BIN and $APP_ID.desktop"
