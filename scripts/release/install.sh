#!/bin/sh
# Installs wubble for the current user from its GitHub releases, or removes it.
#
#   curl -fsSL https://raw.githubusercontent.com/xuckless/wubble/main/scripts/release/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --uninstall
#
# Linux: the bundle goes to ~/.local/share/wubble/versions/<version>/, `current` links to it,
# ~/.local/bin/wubble links to current/wubble, and a desktop entry and icons go to the XDG dirs.
# No sudo. macOS: wubble.app goes to /Applications (or ~/Applications when that is not
# writable), unmodified, as signed and notarised. Running it again installs the newest release
# (an installed version is not unpacked twice). wubble updates itself from then on.
#
# Every download is checked against its .sha256; its minisign signature too when `minisign` is
# installed. Layout and formats: docs/release.md.
set -eu

# The key the releases are signed with: the same as the app's (crates/wubble-core/src/update/key.rs;
# `cargo test -p wubble-release` checks they agree). The all-zero value means no key yet.
PUBLIC_KEY='RWQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
PLACEHOLDER='RWQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'

RELEASES=${WUBBLE_RELEASES:-https://github.com/xuckless/wubble/releases/download}

usage() {
    cat <<EOF
usage: install.sh [--channel preview|stable] [--version V] [--uninstall]

  --channel C   follow the preview (default) or stable releases
  --version V   install this version instead of the channel's newest
  --uninstall   remove wubble (your profile is kept)

WUBBLE_RELEASES overrides the download base ($RELEASES), for testing
against a local server or a file:// folder of the same shape.
EOF
}

say() { printf 'wubble: %s\n' "$*"; }
die() {
    printf 'wubble: %s\n' "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

channel=preview
version=
action=install
while [ $# -gt 0 ]; do
    case $1 in
    --channel)
        channel=${2:?--channel needs preview or stable}
        shift 2
        ;;
    --version)
        version=${2:?--version needs a version}
        shift 2
        ;;
    --uninstall)
        action=uninstall
        shift
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        exit 2
        ;;
    esac
done
case $channel in preview | stable) ;; *) die "unknown channel $channel" ;; esac

os=$(uname -s)
case $(uname -m) in
x86_64 | amd64) arch=x86_64 ;;
arm64 | aarch64) arch=aarch64 ;;
*) die "no wubble build for the processor $(uname -m)" ;;
esac
case $os in
Linux)
    [ "$arch" = x86_64 ] || die "no wubble build for Linux on $arch yet"
    platform=linux-x86_64
    ;;
Darwin) platform=macos-$arch ;;
*) die "no wubble build for $os" ;;
esac

[ -n "${HOME:-}" ] || die "HOME is not set"
data_home=${XDG_DATA_HOME:-$HOME/.local/share}
root=$data_home/wubble
launcher=$HOME/.local/bin/wubble
apps=$data_home/applications
icons=$data_home/icons/hicolor
mac_support="$HOME/Library/Application Support/wubble"

fetch() { # url file
    if have curl; then
        curl -fsSL --proto '=https,http,file' -o "$2" "$1"
    elif have wget; then
        wget -q -O "$2" "$1"
    else
        die "needs curl or wget"
    fi
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "needs sha256sum or shasum"
    fi
}

refresh_desktop() {
    if have update-desktop-database; then update-desktop-database "$apps" 2>/dev/null || true; fi
    if have gtk-update-icon-cache; then gtk-update-icon-cache -q -t -f "$icons" 2>/dev/null || true; fi
}

# The mac app's place: /Applications when the user may write there.
mac_dir() {
    if [ -w /Applications ]; then echo /Applications; else echo "$HOME/Applications"; fi
}

uninstall() {
    case $os in
    Linux)
        if [ -L "$launcher" ]; then
            case $(readlink "$launcher") in
            "$root"/*)
                rm -f "$launcher"
                say "removed $launcher"
                ;;
            *) say "left $launcher: it is not wubble's link" ;;
            esac
        fi
        if [ -f "$apps/wubble.desktop" ]; then
            rm -f "$apps/wubble.desktop"
            say "removed $apps/wubble.desktop"
        fi
        for f in "$icons"/*/apps/wubble*.png "$icons"/*/apps/wubble*.svg; do
            if [ -e "$f" ]; then
                rm -f "$f"
                say "removed $f"
            fi
        done
        refresh_desktop
        for p in "$root/current" "$root/current.new" "$root/versions" "$root/updates"; do
            if [ -e "$p" ] || [ -L "$p" ]; then
                rm -rf "$p"
                say "removed $p"
            fi
        done
        say "your profile in $root is kept; remove it by hand to forget everything"
        ;;
    Darwin)
        for d in /Applications "$HOME/Applications"; do
            if [ -d "$d/wubble.app" ]; then
                rm -rf "$d/wubble.app"
                say "removed $d/wubble.app"
            fi
        done
        if [ -d "$mac_support/updates" ]; then
            rm -rf "$mac_support/updates"
            say "removed $mac_support/updates"
        fi
        say "your profile in $mac_support is kept; remove it by hand to forget everything"
        ;;
    esac
}

if [ "$action" = uninstall ]; then
    uninstall
    exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/wubble-install.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' INT TERM

if [ -z "$version" ]; then
    say "asking for the newest $channel release"
    fetch "$RELEASES/updates/$channel.json" "$tmp/manifest.json" || die "cannot fetch the $channel manifest"
    version=$(tr -d '\n\r' <"$tmp/manifest.json" | sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
    [ -n "$version" ] || die "the manifest names no version"
fi
case $version in *[!0-9A-Za-z.+-]* | '') die "not a version: $version" ;; esac

name=wubble-$version-$platform.tar.xz
url=$RELEASES/v$version/$name
say "downloading $url"
fetch "$url" "$tmp/$name" || die "no wubble $version for $platform ($url)"
fetch "$url.sha256" "$tmp/$name.sha256" || die "no checksum for $name"
want=$(cut -d' ' -f1 <"$tmp/$name.sha256")
got=$(sha256_of "$tmp/$name")
[ "$want" = "$got" ] || die "the checksum of $name does not match ($got, want $want)"
say "checksum ok"

if [ "$PUBLIC_KEY" = "$PLACEHOLDER" ]; then
    say "warning: this script has no signing key yet; the signature was not checked"
elif have minisign; then
    fetch "$url.minisig" "$tmp/$name.minisig" || die "no signature for $name"
    minisign -V -q -P "$PUBLIC_KEY" -m "$tmp/$name" -x "$tmp/$name.minisig" ||
        die "the signature of $name does not match: not installed"
    say "signature ok"
else
    say "minisign is not installed, so the signature was not checked (only the checksum);"
    say "install minisign (dnf, apt or brew: minisign) to have it checked"
fi

install_linux() {
    have xz || die "needs xz to unpack (install xz-utils or xz)"
    dest=$root/versions/$version
    if [ -f "$dest/release.json" ]; then
        say "wubble $version is already in $dest"
    else
        mkdir -p "$root/versions"
        partial=$dest.partial
        rm -rf "$partial"
        mkdir -p "$partial"
        say "unpacking into $dest"
        xz -dc "$tmp/$name" | tar -xf - -C "$partial" --strip-components=1
        [ -f "$partial/release.json" ] || die "the archive holds no wubble bundle"
        rm -rf "$dest"
        mv "$partial" "$dest"
    fi
    # current -> versions/<version>, replaced by a rename where mv can (GNU mv -T)
    rm -rf "$root/current.new"
    ln -s "versions/$version" "$root/current.new"
    if ! mv -T "$root/current.new" "$root/current" 2>/dev/null; then
        rm -rf "$root/current"
        mv "$root/current.new" "$root/current"
    fi
    say "current is wubble $version"
    mkdir -p "$(dirname "$launcher")" "$root/updates"
    ln -sfn "$root/current/wubble" "$launcher"
    say "linked $launcher"
    if [ -d "$dest/share/icons/hicolor" ]; then
        (cd "$dest/share/icons/hicolor" && find . -type f) | while read -r f; do
            mkdir -p "$icons/$(dirname "$f")"
            cp "$dest/share/icons/hicolor/$f" "$icons/$f"
        done
        say "installed the icons under $icons"
    fi
    if [ -f "$dest/share/applications/wubble.desktop" ]; then
        mkdir -p "$apps"
        sed "s|^Exec=wubble|Exec=$launcher|" "$dest/share/applications/wubble.desktop" >"$apps/wubble.desktop"
        say "installed $apps/wubble.desktop"
    fi
    refresh_desktop
    case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) say "note: $HOME/.local/bin is not on your PATH; add it to start wubble by name" ;;
    esac
    say "done: run wubble, or find it in your apps"
}

install_mac() {
    mkdir -p "$tmp/app"
    say "unpacking"
    tar -xJf "$tmp/$name" -C "$tmp/app"
    [ -f "$tmp/app/wubble.app/Contents/Resources/release.json" ] || die "the archive holds no wubble.app"
    dir=$(mac_dir)
    mkdir -p "$dir"
    target=$dir/wubble.app
    if [ -d "$target" ]; then
        mv "$target" "$tmp/old.app"
        if ! mv "$tmp/app/wubble.app" "$target"; then
            mv "$tmp/old.app" "$target"
            die "cannot put wubble.app into $dir"
        fi
        say "replaced $target"
    else
        mv "$tmp/app/wubble.app" "$target"
        say "installed $target"
    fi
    mkdir -p "$mac_support/updates"
    say "done: open wubble from $dir"
}

case $os in
Linux) install_linux ;;
Darwin) install_mac ;;
esac
