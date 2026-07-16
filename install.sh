#!/bin/sh

set -eu

repository="AKMessi/pactrail"
version="${PACTRAIL_VERSION:-latest}"
install_dir="${PACTRAIL_INSTALL_DIR:-${HOME}/.local/bin}"

fail() {
    printf 'pactrail installer: %s\n' "$1" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"

case "$version" in
    latest) ;;
    v[0-9]*)
        case "$version" in
            *[!A-Za-z0-9._-]*) fail "PACTRAIL_VERSION contains unsupported characters" ;;
        esac
        ;;
    *) fail "PACTRAIL_VERSION must be 'latest' or a v-prefixed release tag" ;;
esac

operating_system=$(uname -s)
architecture=$(uname -m)
case "${operating_system}/${architecture}" in
    Linux/x86_64 | Linux/amd64)
        asset="pactrail-linux-x86_64.tar.gz"
        ;;
    Darwin/arm64 | Darwin/aarch64)
        asset="pactrail-macos-aarch64.tar.gz"
        ;;
    *)
        fail "no prebuilt binary for ${operating_system}/${architecture}; use the Cargo install command from the README"
        ;;
esac

if [ "$version" = "latest" ]; then
    release_base="https://github.com/${repository}/releases/latest/download"
else
    release_base="https://github.com/${repository}/releases/download/${version}"
fi

temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/pactrail-install.XXXXXX")
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM
archive="${temporary_dir}/${asset}"
checksums="${temporary_dir}/SHA256SUMS"

printf 'Downloading Pactrail %s for %s/%s...\n' "$version" "$operating_system" "$architecture"
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    "${release_base}/${asset}" --output "$archive"
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    "${release_base}/SHA256SUMS" --output "$checksums"

expected=$(awk -v asset="$asset" '
    {
        name = $2
        sub(/^\*/, "", name)
        sub(/^artifacts\//, "", name)
        if (name == asset) {
            print tolower($1)
            exit
        }
    }
' "$checksums")
case "$expected" in
    '' | *[!0-9a-f]*) fail "release checksum manifest has no valid entry for ${asset}" ;;
esac
[ "${#expected}" -eq 64 ] || fail "release checksum for ${asset} is malformed"

if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$archive" | awk '{ print tolower($1) }')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$archive" | awk '{ print tolower($1) }')
else
    fail "sha256sum or shasum is required to verify the release"
fi
[ "$actual" = "$expected" ] || fail "SHA-256 verification failed for ${asset}"

unpack_dir="${temporary_dir}/unpack"
mkdir -p "$unpack_dir"
tar -xzf "$archive" -C "$unpack_dir"
[ -f "${unpack_dir}/pactrail" ] || fail "release archive does not contain pactrail"

mkdir -p "$install_dir"
if command -v install >/dev/null 2>&1; then
    install -m 0755 "${unpack_dir}/pactrail" "${install_dir}/pactrail"
else
    cp "${unpack_dir}/pactrail" "${install_dir}/pactrail"
    chmod 0755 "${install_dir}/pactrail"
fi

printf 'Installed %s\n' "$("${install_dir}/pactrail" --version)"
case ":${PATH}:" in
    *":${install_dir}:"*) ;;
    *)
        printf 'Add %s to PATH to run pactrail from any terminal.\n' "$install_dir"
        ;;
esac
