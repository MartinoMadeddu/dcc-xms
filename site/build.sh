#!/bin/sh
# Assemble the website into a folder: the page, the program's splash and
# icon, and the screenshots from docs/. Run from the root of the repository:
#
#     sh site/build.sh _site
#
# then open _site/index.html.
set -e
out="${1:-_site}"
rm -rf "$out"
mkdir -p "$out/shots"
cp site/index.html "$out/"
cp assets/splash.png assets/icon.png "$out/"
cp docs/*.png "$out/shots/"
# No Jekyll processing: files are served as they are.
touch "$out/.nojekyll"
echo "site written to $out"
