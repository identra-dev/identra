#!/bin/sh
# Install Identra.
#
#   curl -fsSL https://identra.dev/install.sh | sh
#
# The site serves a copy of this exact file. That copy needs no maintenance per release, which is
# the point of resolving the version at runtime rather than baking one in: a new release is picked
# up by every existing copy of this script the moment it is published.
#
# One download, and then you are done: the release bundle carries its own runtime and its own
# embedding model, so nothing here fetches a second thing on first launch or asks you to install a
# toolchain first. That is the whole design goal of this script. If you ever see it reach for a
# package manager, something upstream has regressed.
#
# POSIX sh on purpose. `curl | sh` runs under whatever /bin/sh is, which on Debian and Ubuntu is
# dash, and a bashism here fails on the majority of the machines this is aimed at.
set -eu

REPO="identra-dev/identra"
# Overridable so the same script installs a specific build: IDENTRA_VERSION=v0.1.1 sh install.sh
VERSION="${IDENTRA_VERSION:-}"

BIN_DIR="${IDENTRA_BIN_DIR:-$HOME/.local/bin}"
APP_DIR="${IDENTRA_APP_DIR:-$HOME/.local/share/identra/app}"

say() { printf '  %s\n' "$*"; }
die() { printf '\nidentra: %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "need $1 on PATH to install"; }

# Installing a system package needs root. sudo reads its password straight from the terminal rather
# than from stdin, so this still prompts correctly when the script itself arrived down a pipe from
# curl. Already root (a container, a provisioning script) skips it.
run_root() {
  if [ "$(id -u)" = "0" ]; then
    "$@"
  else
    need sudo
    sudo "$@"
  fi
}

# --- what are we installing onto -------------------------------------------------------------

os=$(uname -s)
arch=$(uname -m)

case "$os" in
  Linux)
    # The Linux bundle is built on an x86_64 runner. An arm64 Linux box gets a clear no rather than
    # a binary it cannot run.
    [ "$arch" = "x86_64" ] || die "no Linux build for $arch yet — build from source: https://github.com/$REPO"
    # A real system package, so Identra is in the app menu, `apt remove` takes it away again, and
    # the system libraries it links against are resolved by the thing that owns them. The AppImage
    # is the fallback for distros that are neither, not the default.
    if command -v apt-get >/dev/null 2>&1; then
      kind=deb; asset='\.deb$'
    elif command -v dnf >/dev/null 2>&1; then
      kind=rpm; asset='\.rpm$'
    else
      kind=appimage; asset='\.AppImage$'
    fi
    ;;
  Darwin)
    # Apple Silicon only, and deliberately: the embedding runtime ships no prebuilt for Intel macs,
    # so there is no x86_64 half to link. See .github/workflows/build.yml.
    [ "$arch" = "arm64" ] || die "no macOS build for $arch — Identra needs an Apple Silicon mac"
    kind=dmg; asset='\.dmg$'
    ;;
  *) die "unsupported OS: $os (Linux and macOS only)" ;;
esac

need curl

# --- which release ----------------------------------------------------------------------------

if [ -z "$VERSION" ]; then
  # No jq. Asking someone to install a JSON parser before they can install the app is exactly the
  # extra download this script exists to avoid.
  VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$VERSION" ] || die "could not reach the GitHub API to find the latest release — try again, or set IDENTRA_VERSION"
fi

# Ask the release for its own asset URL rather than guessing the filename. Bundle names carry the
# version and the architecture in a format the packager owns, so a hardcoded guess here breaks on
# the day that format changes and nobody notices until an install 404s.
#
# The pattern is anchored to the end of the URL. Every bundle has a `.sig` sibling next to it for
# the updater, and an unanchored match picks whichever of the pair the API happened to list first.
url=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/tags/$VERSION" 2>/dev/null \
  | sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p' | grep "$asset" | head -n 1)
[ -n "$url" ] || die "release $VERSION has no build for $os $arch"

say "Identra $VERSION"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

file="$tmp/${url##*/}"
say "downloading $(basename "$file")"
curl -fL --progress-bar "$url" -o "$file" || die "download failed"

# --- install ------------------------------------------------------------------------------------

if [ "$kind" = "deb" ]; then
  # apt-get rather than dpkg -i. A .deb declares the system libraries it links against (webkit2gtk
  # among them) and dpkg will not fetch them, so `dpkg -i` leaves a half-configured package and an
  # app that dies on a missing .so. apt resolves them, which is the one place this script does pull
  # something else down — and they are OS libraries, not any part of Identra.
  say "installing (needs your password for dpkg)"
  DEBIAN_FRONTEND=noninteractive run_root apt-get install -y "$file" || die "apt-get could not install the package"
  say "installed"
  printf '\nOpen Identra from your app menu, or run: identra-desktop\n'
  exit 0
fi

if [ "$kind" = "rpm" ]; then
  say "installing (needs your password for dnf)"
  run_root dnf install -y "$file" || die "dnf could not install the package"
  say "installed"
  printf '\nOpen Identra from your app menu, or run: identra-desktop\n'
  exit 0
fi

if [ "$kind" = "dmg" ]; then
  mount=$(hdiutil attach -nobrowse -readonly "$file" | grep '/Volumes/' | sed -n 's/.*\(\/Volumes\/.*\)/\1/p' | head -n 1)
  [ -n "$mount" ] || die "could not mount the disk image"
  # A copy that leaves the old app in place merges two versions inside one bundle, which is how you
  # get a Frankenstein install that launches an old binary against new resources.
  rm -rf "/Applications/Identra.app"
  cp -R "$mount/Identra.app" /Applications/ || { hdiutil detach "$mount" >/dev/null 2>&1; die "copy to /Applications failed"; }
  hdiutil detach "$mount" >/dev/null 2>&1 || true
  # curl does not set the quarantine bit, so this is usually a no-op. It is here for the case where
  # someone downloaded the script or the dmg through a browser first.
  xattr -dr com.apple.quarantine /Applications/Identra.app 2>/dev/null || true
  say "installed to /Applications/Identra.app"
  printf '\nOpen it from Launchpad, or: open -a Identra\n'
  exit 0
fi

# A Linux box with neither apt nor dnf. The AppImage is unpacked rather than kept whole, because a
# type-2 AppImage mounts itself with libfuse2 and current distros ship only fuse3 — so the
# single-file version fails to start on a large share of exactly the machines it is meant to be the
# portable option for. `--appimage-extract` is handled by the runtime itself and needs no FUSE at
# all, which turns the most common Linux install failure into something that cannot happen.
chmod +x "$file"
say "unpacking"
( cd "$tmp" && "$file" --appimage-extract >/dev/null 2>&1 ) || die "could not unpack the AppImage"
[ -x "$tmp/squashfs-root/AppRun" ] || die "unpacked bundle has no AppRun — the release build is broken"

rm -rf "$APP_DIR"
mkdir -p "$(dirname "$APP_DIR")" "$BIN_DIR"
mv "$tmp/squashfs-root" "$APP_DIR"

ln -sf "$APP_DIR/AppRun" "$BIN_DIR/identra"

# A desktop entry, so Identra is in the launcher and not only on the PATH. Written here rather than
# copied out of the bundle: the bundled one has a relative Exec that only resolves inside a mounted
# AppImage, and rewriting someone else's ini file is more code than these seven lines.
apps="$HOME/.local/share/applications"
icon=$(find "$APP_DIR" -maxdepth 1 -name '*.png' | head -n 1)
mkdir -p "$apps"
cat > "$apps/identra.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Identra
Comment=A canvas for your coding agents
Exec=$BIN_DIR/identra %U
Icon=${icon:-identra}
Categories=Development;
Terminal=false
EOF
update-desktop-database "$apps" >/dev/null 2>&1 || true

say "installed to $APP_DIR"

case ":$PATH:" in
  *":$BIN_DIR:"*) printf '\nRun it: identra\n' ;;
  *) printf '\nRun it: %s/identra\n(%s is not on your PATH; add it there to run Identra by name.)\n' "$BIN_DIR" "$BIN_DIR" ;;
esac
