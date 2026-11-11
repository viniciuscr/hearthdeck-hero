#!/usr/bin/env bash
# Builds Hearthdeck Hero in release mode and installs it for the current user.
# It is a separate app from Heroic/Heronite with its own data folder
# (~/.local/share/hearthdeck); the first start copies an existing Heroic setup.
#   binary   ~/.local/opt/hearthdeck-hero/hearthdeck-hero
#   launcher ~/.local/bin/hearthdeck-hero
#   desktop  ~/.local/share/applications/app.hearthdeck.Hero.desktop
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${HEARTHDECK_HERO_PREFIX:-${HERONITE_RS_PREFIX:-$HOME/.local/opt/hearthdeck-hero}}"
BIN_DIR="$HOME/.local/bin"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
ICON_SRC="$ROOT/packaging/linux/app.hearthdeck.Hero.png"

cd "$ROOT"
cargo build --release -p heronite -p heronite-epic-login

mkdir -p "$PREFIX" "$BIN_DIR" "$APP_DIR"
install -m 755 target/release/hearthdeck-hero "$PREFIX/hearthdeck-hero.new"
mv -f "$PREFIX/hearthdeck-hero.new" "$PREFIX/hearthdeck-hero"
install -m 755 target/release/hearthdeck-hero-epic-login "$PREFIX/hearthdeck-hero-epic-login"
ln -sf "$PREFIX/hearthdeck-hero" "$BIN_DIR/hearthdeck-hero"

ICON_NAME=app.hearthdeck.Hero
if [[ -f "$ICON_SRC" ]]; then
  mkdir -p "$ICON_DIR"
  install -m 644 "$ICON_SRC" "$ICON_DIR/$ICON_NAME.png"
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -f -t "${ICON_DIR%/256x256/apps}" || true
fi

cat > "$APP_DIR/$ICON_NAME.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Hearthdeck Hero (Dev)
Comment=Game Collection and Watch
Exec=$PREFIX/hearthdeck-hero %U
Icon=$ICON_NAME
Terminal=false
Categories=Game;
MimeType=x-scheme-handler/hearthdeck;
StartupWMClass=$ICON_NAME
DESKTOP

command -v xdg-mime >/dev/null && xdg-mime default "$ICON_NAME.desktop" x-scheme-handler/hearthdeck >/dev/null 2>&1 || true
command -v update-desktop-database >/dev/null && update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
echo "Installed hearthdeck-hero to $PREFIX"
