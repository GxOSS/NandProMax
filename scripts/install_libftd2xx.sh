#!/usr/bin/env bash
set -euo pipefail

die() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

uname_s="$(uname -s)"
uname_m="$(uname -m)"

os=""
case "$uname_s" in
  Linux) os="linux" ;;
  Darwin) os="macos" ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT) os="windows" ;;
  *) die "unsupported OS: uname -s=$uname_s" ;;
esac

arch=""
case "$uname_m" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="arm64" ;;
  *) die "unsupported architecture: uname -m=$uname_m" ;;
esac

DEST_PREFIX="${DEST_PREFIX:-/usr/local}"
DEST_LIB_DIR="${DEST_LIB_DIR:-$DEST_PREFIX/lib}"

URL_LINUX_X86_64="${URL_LINUX_X86_64:-https://ftdichip.com/wp-content/uploads/2025/11/libftd2xx-linux-x86_64-1.4.34.tgz}"
URL_LINUX_ARM64="${URL_LINUX_ARM64:-https://ftdichip.com/wp-content/uploads/2025/11/libftd2xx-linux-arm-v8-1.4.34.tgz}"
URL_MACOS_X86_64="${URL_MACOS_X86_64:-https://ftdichip.com/wp-content/uploads/2024/04/D2XX1.4.30.dmg}"
URL_MACOS_ARM64="${URL_MACOS_ARM64:-https://ftdichip.com/wp-content/uploads/2024/04/D2XX1.4.30.dmg}"
URL_WINDOWS_X86_64="${URL_WINDOWS_X86_64:-https://ftdichip.com/wp-content/uploads/2025/03/CDM-v2.12.36.20-WHQL-Certified.zip}"
URL_WINDOWS_ARM64="${URL_WINDOWS_ARM64:-https://ftdichip.com/wp-content/uploads/2025/03/CDM-v2.12.36.20-for-ARM64-WHQL-Certified.zip}"

url=""
case "${os}_${arch}" in
  linux_x86_64) url="$URL_LINUX_X86_64" ;;
  linux_arm64) url="$URL_LINUX_ARM64" ;;
  macos_x86_64) url="$URL_MACOS_X86_64" ;;
  macos_arm64) url="$URL_MACOS_ARM64" ;;
  windows_x86_64) url="$URL_WINDOWS_X86_64" ;;
  windows_arm64) url="$URL_WINDOWS_ARM64" ;;
  *) die "unsupported target: ${os}_${arch}" ;;
esac

if [[ -z "${url}" ]]; then
  die "no download URL configured for ${os}_${arch}. Set one of: URL_LINUX_X86_64, URL_LINUX_ARM64, URL_MACOS_X86_64, URL_MACOS_ARM64, URL_WINDOWS_X86_64, URL_WINDOWS_ARM64"
fi

fetch() {
  local out="$1"
  local user_agent="${HTTP_USER_AGENT:-Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0 Safari/537.36}"
  local referer="${HTTP_REFERER:-https://ftdichip.com/}"
  if [[ "$url" == file://* ]]; then
    local src="${url#file://}"
    [[ -f "$src" ]] || die "file URL does not exist: $url"
    cp -f "$src" "$out"
    return 0
  fi
  if [[ "$url" != http://* && "$url" != https://* ]]; then
    [[ -f "$url" ]] || die "URL is not http(s) and file does not exist: $url"
    cp -f "$url" "$out"
    return 0
  fi
  if command -v curl >/dev/null 2>&1; then
    curl -fSL --retry 5 --retry-delay 1 --connect-timeout 20 --max-time 600 \
      -H "User-Agent: $user_agent" -e "$referer" \
      "$url" -o "$out" || {
        if curl -sSIL -H "User-Agent: $user_agent" -e "$referer" "$url" >/dev/null 2>&1; then
          die "download failed: $url"
        fi
        die "download failed (possible 403). If ftdichip blocks CI traffic, mirror the file elsewhere and override the URL_* variables."
      }
  elif command -v wget >/dev/null 2>&1; then
    wget --user-agent="$user_agent" --referer="$referer" -qO "$out" "$url" || die "download failed (possible 403): $url"
  else
    die "need curl or wget to download"
  fi
}

tmpdir="$(mktemp -d)"

need_cmd uname
need_cmd mktemp
need_cmd find

artifact="$tmpdir/artifact"
fetch "$artifact"

extract_dir="$tmpdir/extract"
mkdir -p "$extract_dir"

roots=()
roots+=("$extract_dir")

try_extract_zip() {
  command -v unzip >/dev/null 2>&1 || return 1
  unzip -tq "$artifact" >/dev/null 2>&1 || return 1
  unzip -q "$artifact" -d "$extract_dir"
  return 0
}

try_extract_tar_gz() {
  command -v tar >/dev/null 2>&1 || return 1
  tar -tzf "$artifact" >/dev/null 2>&1 || return 1
  tar -xzf "$artifact" -C "$extract_dir"
  return 0
}

try_extract_tar_bz2() {
  command -v tar >/dev/null 2>&1 || return 1
  tar -tjf "$artifact" >/dev/null 2>&1 || return 1
  tar -xjf "$artifact" -C "$extract_dir"
  return 0
}

try_extract_tar_xz() {
  command -v tar >/dev/null 2>&1 || return 1
  tar -tJf "$artifact" >/dev/null 2>&1 || return 1
  tar -xJf "$artifact" -C "$extract_dir"
  return 0
}

try_extract_gzip_single() {
  command -v gzip >/dev/null 2>&1 || return 1
  gzip -t "$artifact" >/dev/null 2>&1 || return 1
  gzip -dc "$artifact" >"$extract_dir/unpacked"
  return 0
}

mounted=0
mountdir="$tmpdir/mount"
payload_dir="$tmpdir/payload"

try_extract_dmg_macos() {
  [[ "$os" == "macos" ]] || return 1
  command -v hdiutil >/dev/null 2>&1 || return 1
  mkdir -p "$mountdir"
  hdiutil attach "$artifact" -nobrowse -mountpoint "$mountdir" -quiet >/dev/null 2>&1 || return 1
  mounted=1
  roots+=("$mountdir")

  command -v pkgutil >/dev/null 2>&1 || return 0
  local pkg
  pkg="$(find "$mountdir" -type f -name '*.pkg' -print -quit 2>/dev/null || true)"
  [[ -n "$pkg" ]] || return 0

  local expanded="$tmpdir/pkg-expanded"
  pkgutil --expand-full "$pkg" "$expanded" >/dev/null 2>&1 || return 0

  local payload
  payload="$(find "$expanded" -type f -name 'Payload' -print -quit 2>/dev/null || true)"
  [[ -n "$payload" ]] || return 0

  command -v cpio >/dev/null 2>&1 || return 0
  mkdir -p "$payload_dir"
  (cd "$payload_dir" && gzip -dc "$payload" | cpio -idmu >/dev/null 2>&1) || true
  roots+=("$payload_dir")
  return 0
}

cleanup() {
  if [[ "$mounted" -eq 1 ]]; then
    hdiutil detach "$mountdir" -quiet >/dev/null 2>&1 || true
  fi
  rm -rf "$tmpdir"
}
trap cleanup EXIT

if [[ "$url" == *.dmg ]] && try_extract_dmg_macos; then
  :
elif try_extract_zip; then
  :
elif try_extract_tar_gz; then
  :
elif try_extract_tar_bz2; then
  :
elif try_extract_tar_xz; then
  :
elif try_extract_gzip_single; then
  :
fi

pick_first() {
  local pattern="$1"
  local root
  local found=""
  for root in "${roots[@]}"; do
    found="$(find "$root" -type f -name "$pattern" -print -quit 2>/dev/null || true)"
    if [[ -n "$found" ]]; then
      echo "$found"
      return 0
    fi
  done
  return 1
}

lib_path=""
case "$os" in
  linux)
    lib_path="$(pick_first 'libftd2xx.so')"
    if [[ -z "$lib_path" ]]; then
      lib_path="$(find "${roots[@]}" -type f -name 'libftd2xx.so.*' -print 2>/dev/null | sort | tail -n 1 || true)"
    fi
    ;;
  macos)
    lib_path="$(pick_first 'libftd2xx.dylib')"
    ;;
  windows)
    lib_path="$(pick_first 'ftd2xx.dll')"
    if [[ -z "$lib_path" ]]; then
      lib_path="$(pick_first 'libftd2xx.dll')"
    fi
    ;;
esac

if [[ -z "$lib_path" || ! -f "$lib_path" ]]; then
  die "downloaded artifact does not contain an expected library file for ${os}_${arch}"
fi

sudo_prefix=()
if [[ "$(id -u)" -ne 0 ]]; then
  sudo_prefix=(sudo)
fi

"${sudo_prefix[@]}" mkdir -p "$DEST_LIB_DIR"

base="$(basename "$lib_path")"
dest="$DEST_LIB_DIR/$base"
"${sudo_prefix[@]}" cp -f "$lib_path" "$dest"

if [[ "$os" == "linux" ]]; then
  if [[ "$base" == libftd2xx.so.* ]]; then
    "${sudo_prefix[@]}" ln -sf "$base" "$DEST_LIB_DIR/libftd2xx.so"
  fi
  if command -v ldconfig >/dev/null 2>&1; then
    "${sudo_prefix[@]}" ldconfig || true
  fi
fi

echo "installed: $dest"
