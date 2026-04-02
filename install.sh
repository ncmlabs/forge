#!/bin/sh
# FORGE installer — downloads the latest release binary for your platform.
# Usage: curl -fsSL https://raw.githubusercontent.com/ncmlabs/forge/main/install.sh | sh

set -eu

REPO="ncmlabs/forge"
INSTALL_DIR="${FORGE_INSTALL_DIR:-$HOME/.forge/bin}"

main() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os_target="unknown-linux-gnu" ;;
        Darwin) os_target="apple-darwin" ;;
        *)      echo "Error: unsupported OS: $os" >&2; exit 1 ;;
    esac

    case "$arch" in
        x86_64|amd64)   arch_target="x86_64" ;;
        aarch64|arm64)   arch_target="aarch64" ;;
        *)               echo "Error: unsupported architecture: $arch" >&2; exit 1 ;;
    esac

    target="${arch_target}-${os_target}"

    # Get latest release tag
    if command -v curl >/dev/null 2>&1; then
        tag="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"tag_name": "\(.*\)".*/\1/')"
    elif command -v wget >/dev/null 2>&1; then
        tag="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"tag_name": "\(.*\)".*/\1/')"
    else
        echo "Error: curl or wget required" >&2
        exit 1
    fi

    if [ -z "$tag" ]; then
        echo "Error: could not determine latest release" >&2
        exit 1
    fi

    echo "Installing FORGE ${tag} for ${target}..."

    archive="forge-${target}.tar.gz"
    checksum_file="${archive}.sha256"
    url="https://github.com/${REPO}/releases/download/${tag}/${archive}"
    checksum_url="https://github.com/${REPO}/releases/download/${tag}/${checksum_file}"

    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    # Download archive and checksum
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$tmpdir/$archive"
        curl -fsSL "$checksum_url" -o "$tmpdir/$checksum_file"
    else
        wget -q "$url" -O "$tmpdir/$archive"
        wget -q "$checksum_url" -O "$tmpdir/$checksum_file"
    fi

    # Verify checksum
    cd "$tmpdir"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "$checksum_file"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "$checksum_file"
    else
        echo "Warning: could not verify checksum (no sha256sum or shasum found)" >&2
    fi

    # Extract and install
    tar xzf "$archive"
    mkdir -p "$INSTALL_DIR"
    mv forge "$INSTALL_DIR/forge"
    chmod +x "$INSTALL_DIR/forge"

    echo ""
    echo "FORGE installed to ${INSTALL_DIR}/forge"

    # Check if install dir is in PATH
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            echo ""
            echo "Add FORGE to your PATH by adding this to your shell profile:"
            echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            ;;
    esac

    echo ""
    echo "Run 'forge --help' to get started."
}

main
