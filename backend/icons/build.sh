#!/bin/sh
# Makes every icon of the app and the website from source.svg, and the
# social card from social-card.html. Run it from any folder on macOS with
# Google Chrome installed.
set -eu

icons=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$icons/../.." && pwd)
docs="$root/docs"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

render() { # render <svg> <out dir>
  (cd "$root" && pnpm --silent tauri icon "$1" -o "$2" >/dev/null 2>&1)
}

# The app icons keep the margin of the macOS icon grid.
render "$icons/source.svg" "$work/app"
for f in "$icons"/*.png "$icons"/icon.icns "$icons"/icon.ico; do
  cp "$work/app/$(basename "$f")" "$f"
done

# Web pages show the tile without the margin and the shadow, so the art
# is larger at small sizes.
sed -e 's/viewBox="0 0 1024 1024"/viewBox="100 100 824 824"/' \
  -e 's/ filter="url(#shadow)"//' "$icons/source.svg" >"$work/tile.svg"
render "$work/tile.svg" "$work/tile"
cp "$work/tile/32x32.png" "$docs/favicon.png"
cp "$work/tile/icon.ico" "$docs/favicon.ico"
sips -z 256 256 "$work/tile/icon.png" --out "$docs/assets/icon.png" >/dev/null

# iOS rounds the corners itself and fills transparent pixels with black,
# so the touch icon is a full square.
sed -e 's/rx="185"/rx="0"/' -e '/stroke-opacity="0.08"/d' \
  "$work/tile.svg" >"$work/touch.svg"
render "$work/touch.svg" "$work/touch"
sips -z 180 180 "$work/touch/icon.png" --out "$docs/assets/apple-touch-icon.png" >/dev/null

"$chrome" --headless=new --hide-scrollbars --force-device-scale-factor=1 \
  --window-size=1280,640 --screenshot="$docs/assets/social-card.png" \
  "file://$icons/social-card.html" 2>/dev/null
