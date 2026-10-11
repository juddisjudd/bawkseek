#!/bin/sh
# Writes PKGBUILD and .SRCINFO for bawkseek-bin. Usage: render.sh <version> <sha256 of the release tar.gz> <output dir>
set -e
version="$1"
sha="$2"
out="${3:-.}"
here="$(cd "$(dirname "$0")" && pwd)"

sed -e "s/@VERSION@/${version}/" -e "s/@SHA256@/${sha}/" "$here/PKGBUILD.in" > "$out/PKGBUILD"

cp "$here/LICENSE" "$out/LICENSE"

# .SRCINFO mirrors PKGBUILD; written here so CI needs no Arch container for makepkg --printsrcinfo
cat > "$out/.SRCINFO" <<EOF
pkgbase = bawkseek-bin
	pkgdesc = Soulseek client and audio player
	pkgver = ${version}
	pkgrel = 1
	url = https://github.com/juddisjudd/bawkseek
	arch = x86_64
	license = AGPL-3.0-only
	depends = alsa-lib
	depends = gcc-libs
	depends = glibc
	depends = hicolor-icon-theme
	depends = libxcb
	depends = libxkbcommon
	depends = libxkbcommon-x11
	depends = vulkan-icd-loader
	depends = wayland
	optdepends = gnome-keyring: remember your password
	optdepends = kwallet: remember your password
	provides = bawkseek
	conflicts = bawkseek
	options = !strip
	options = !debug
	source = https://github.com/juddisjudd/bawkseek/releases/download/v${version}/bawkseek-v${version}-linux-x64.tar.gz
	sha256sums = ${sha}

pkgname = bawkseek-bin
EOF
