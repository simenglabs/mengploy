#!/bin/sh
# Installer mengploy: curl -fsSL https://raw.githubusercontent.com/simenglabs/mengploy/main/install.sh | sh
# Set MENGPLOY_VERSION=vX.Y.Z untuk versi tertentu.
set -eu

REPOSITORY="${MENGPLOY_REPOSITORY:-simenglabs/mengploy}"
VERSION="${MENGPLOY_VERSION:-latest}"
INSTALL_DIR="${MENGPLOY_INSTALL_DIR:-}"
BINARY_NAME="mengploy"

say() {
    printf '%s\n' "mengploy: $*"
}

fail() {
    printf '%s\n' "mengploy: error: $*" >&2
    exit 1
}

need_command() {
    command -v "$1" >/dev/null 2>&1 || fail "perintah '$1' diperlukan tetapi tidak ditemukan"
}

prompt_yes() {
    if [ "${MENGPLOY_ASSUME_YES:-0}" = "1" ]; then
        return 0
    fi
    if [ ! -t 0 ] && [ ! -r /dev/tty ]; then
        fail "stdin bukan terminal; set MENGPLOY_ASSUME_YES=1 atau jalankan installer dari terminal"
    fi
    printf '%s ' "mengploy: lanjutkan instalasi ke $1? [y/N]" >&2
    if [ -r /dev/tty ]; then
        read -r answer </dev/tty
    else
        read -r answer
    fi
    case "$answer" in
        y|Y|yes|YES|yEs|Yes) return 0 ;;
        *) fail "instalasi dibatalkan" ;;
    esac
}

need_command uname
need_command mktemp
need_command tar
need_command chmod
need_command mkdir
need_command cp
need_command mv
need_command rm
need_command awk
need_command sed
need_command grep
need_command head
need_command curl

OS=$(uname -s)
ARCH=$(uname -m)
case "$OS:$ARCH" in
    Linux:x86_64|Linux:amd64) TARGET="x86_64-unknown-linux-gnu" ;;
    Linux:arm64|Linux:aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
    Darwin:x86_64|Darwin:amd64) TARGET="x86_64-apple-darwin" ;;
    Darwin:arm64|Darwin:aarch64) TARGET="aarch64-apple-darwin" ;;
    *) fail "platform tidak didukung: $OS/$ARCH (target: Linux x86_64/aarch64, macOS x86_64/arm64)" ;;
 esac

case "$OS" in
    Linux)
        if [ "$ARCH" = "aarch64" ]; then TARGET="aarch64-unknown-linux-gnu"; fi
        ;;
esac

if [ -z "$INSTALL_DIR" ]; then
    if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
        INSTALL_DIR=/usr/local/bin
    else
        INSTALL_DIR="${HOME:-.}/.local/bin"
    fi
fi

case "$VERSION" in
    latest)
        API_URL="https://api.github.com/repos/$REPOSITORY/releases/latest"
        VERSION=$(curl -fsSL "$API_URL" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
        [ -n "$VERSION" ] || fail "release stabil terbaru tidak ditemukan di $REPOSITORY"
        ;;
esac

ARCHIVE="$BINARY_NAME-$VERSION-$TARGET.tar.gz"
BASE_URL="https://github.com/$REPOSITORY/releases/download/$VERSION"
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/mengploy-install.XXXXXX")
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

say "platform: $TARGET"
say "versi: $VERSION"
say "lokasi: $INSTALL_DIR/$BINARY_NAME"
prompt_yes "$INSTALL_DIR/$BINARY_NAME"

curl -fL --retry 3 --proto '=https' --tlsv1.2 \
    "$BASE_URL/$ARCHIVE" -o "$TMP_DIR/$ARCHIVE"
curl -fL --retry 3 --proto '=https' --tlsv1.2 \
    "$BASE_URL/checksums.txt" -o "$TMP_DIR/checksums.txt"

EXPECTED=$(awk -v file="$ARCHIVE" '$2 == file || $2 == "*" file { print $1; exit }' "$TMP_DIR/checksums.txt")
[ -n "$EXPECTED" ] || fail "checksum untuk $ARCHIVE tidak ditemukan"

if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL=$(sha256sum "$TMP_DIR/$ARCHIVE" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL=$(shasum -a 256 "$TMP_DIR/$ARCHIVE" | awk '{print $1}')
else
    fail "sha256sum atau shasum diperlukan untuk verifikasi release"
fi

[ "$EXPECTED" = "$ACTUAL" ] || fail "checksum release tidak cocok; file tidak dipasang"

mkdir -p "$TMP_DIR/unpacked" "$INSTALL_DIR"
tar -xzf "$TMP_DIR/$ARCHIVE" -C "$TMP_DIR/unpacked"
[ -f "$TMP_DIR/unpacked/$BINARY_NAME" ] || fail "archive tidak berisi executable $BINARY_NAME"
chmod 0755 "$TMP_DIR/unpacked/$BINARY_NAME"

if [ -e "$INSTALL_DIR/$BINARY_NAME" ]; then
    OLD_MODE=$(umask)
    umask 022
    cp "$TMP_DIR/unpacked/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME.new"
    chmod 0755 "$INSTALL_DIR/$BINARY_NAME.new"
    mv "$INSTALL_DIR/$BINARY_NAME.new" "$INSTALL_DIR/$BINARY_NAME"
    umask "$OLD_MODE"
else
    cp "$TMP_DIR/unpacked/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
    chmod 0755 "$INSTALL_DIR/$BINARY_NAME"
fi

say "berhasil memasang $INSTALL_DIR/$BINARY_NAME ($VERSION)"
case ":${PATH:-}:" in
    *:"$INSTALL_DIR":*) ;;
    *) say "tambahkan $INSTALL_DIR ke PATH jika perintah 'mengploy' belum ditemukan" ;;
esac
