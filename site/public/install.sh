#!/bin/sh
# Installs Trayci on Linux from a GitHub release. Needs only sh, curl or wget,
# and sha256sum or shasum.
#
#   curl -fsSL https://reqhiem.github.io/trayci/install.sh | sh
#
# Environment:
#   TRAYCI_VERSION      exact version to install, e.g. 0.4.5 (default: latest)
#   TRAYCI_FORMAT       deb or appimage (default: deb where apt-get exists)
#   TRAYCI_INSTALL_DIR  where the AppImage goes, as `trayci` (default: ~/.local/bin)
#
# The .deb goes through `sudo apt-get install`, which pulls in WebKitGTK and adds
# the menu entry. The AppImage needs no root, but does need libfuse2. Every
# download is checked against the SHA-256 digest GitHub records for the asset.
set -eu

repo="reqhiem/trayci"

fail() {
  printf '\ntrayci install: %s\n' "$1" >&2
  exit 1
}

step() {
  if "$interactive"; then printf '\r\033[2K  %s%s%s' "$muted" "$1" "$reset" >&2
  else printf '  %s\n' "$1" >&2; fi
}

# Exit 44 on a 404 so callers can tell "no such release" from a network failure.
fetch() {
  if command -v curl >/dev/null 2>&1; then
    status="$(curl -sSL -w '%{http_code}' "$1" -o "$2")" || return 1
    case "$status" in
      2??) return 0 ;;
      404) return 44 ;;
      *) printf 'GET %s returned HTTP %s\n' "$1" "$status" >&2; return 1 ;;
    esac
  elif command -v wget >/dev/null 2>&1; then
    wget -q --server-response "$1" -O "$2" 2>"$2.headers" && rm -f "$2.headers" && return 0
    if grep -q ' 404 ' "$2.headers" 2>/dev/null; then rm -f "$2.headers"; return 44; fi
    cat "$2.headers" >&2; rm -f "$2.headers"; return 1
  else
    fail "curl or wget is required"
  fi
}

mb() {
  tenths=$((($1 * 10 + 524288) / 1048576))
  printf '%s.%s' "$((tenths / 10))" "$((tenths % 10))"
}

# Poll the file written by the downloader; no progress-output parsing or extra request.
download() {
  if ! "$interactive"; then fetch "$1" "$2"; return; fi
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -D "$2.headers" "$1" -o "$2" 2>"$2.errors" &
  else
    wget -q --server-response "$1" -O "$2" 2>"$2.headers" &
  fi
  download_pid=$!
  previous=-1
  cr="$(printf '\r')"
  while kill -0 "$download_pid" 2>/dev/null; do
    bytes=0; total=0
    if [ -f "$2" ]; then bytes="$(wc -c < "$2")"; fi
    if [ -f "$2.headers" ]; then
      while read -r key value; do
        case "$key" in
          HTTP/*) total=0 ;;
          [Cc]ontent-[Ll]ength:) total="${value%"$cr"}" ;;
        esac
      done < "$2.headers"
    fi
    case "$total" in ''|*[!0-9]*) total=0 ;; esac
    if [ "$bytes" -ne "$previous" ]; then
      if [ "$total" -gt 0 ]; then
        percent=$((bytes * 100 / total)); [ "$percent" -le 100 ] || percent=100
        filled=$((percent * 32 / 100)); bar=; rest=; n=0
        while [ "$n" -lt 32 ]; do
          if [ "$n" -lt "$filled" ]; then bar="${bar}■"; else rest="${rest}·"; fi
          n=$((n + 1))
        done
        printf '\r\033[2K  %s%s%s%s%s %3d%%  %s%s / %s MB%s' "$accent" "$bar" "$reset$muted" "$rest" "$reset" "$percent" "$muted" "$(mb "$bytes")" "$(mb "$total")" "$reset" >&2
      else
        printf '\r\033[2K  %sDownloading%s  %s MB' "$muted" "$reset" "$(mb "$bytes")" >&2
      fi
      previous="$bytes"
    fi
    sleep 0.1
  done
  result=0; wait "$download_pid" || result=$?
  download_pid=
  if [ "$result" -ne 0 ]; then
    printf '\n' >&2
    if [ -f "$2.errors" ]; then cat "$2.errors" >&2; else cat "$2.headers" >&2; fi
    return "$result"
  fi
  size="$(mb "$(wc -c < "$2")")"
  printf '\r\033[2K  %s■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■%s 100%%  %s%s / %s MB%s\n' "$accent" "$reset" "$muted" "$size" "$size" "$reset" >&2
  rm -f "$2.headers" "$2.errors"
}

# Everything runs from here, so a download cut short never executes half a script.
main() {
  # ANSI stays on stderr, so `curl ... | sh` still gets progress.
  interactive=false
  if [ -t 2 ] && [ "${TERM:-}" != dumb ]; then interactive=true; fi
  reset= bold= muted= accent= green= yellow=
  if "$interactive" && [ -z "${NO_COLOR:-}" ]; then
    reset="$(printf '\033[0m')"; bold="$(printf '\033[1m')"; muted="$(printf '\033[2m')"
    accent="$(printf '\033[94m')"; green="$(printf '\033[32m')"; yellow="$(printf '\033[33m')"
  fi
  printf '\n  %sTrayci%s %sinstaller%s\n\n' "$bold" "$reset" "$muted" "$reset" >&2

  [ "$(uname -s)" = Linux ] || fail "this script is for Linux; on Windows run: irm https://reqhiem.github.io/trayci/install.ps1 | iex"
  case "$(uname -m)" in
    x86_64 | amd64) ;;
    *) fail "Trayci ships for x86_64 only, not $(uname -m)" ;;
  esac
  if command -v sha256sum >/dev/null 2>&1; then
    checksum() { sha256sum "$1" | cut -d' ' -f1; }
  elif command -v shasum >/dev/null 2>&1; then
    checksum() { shasum -a 256 "$1" | cut -d' ' -f1; }
  else
    fail "sha256sum or shasum is required"
  fi

  format="${TRAYCI_FORMAT:-}"
  if [ -z "$format" ]; then
    if command -v apt-get >/dev/null 2>&1; then format=deb; else format=appimage; fi
  fi
  case "$format" in
    deb) suffix="amd64.deb" ;;
    appimage) suffix="amd64.AppImage" ;;
    *) fail "TRAYCI_FORMAT must be deb or appimage" ;;
  esac
  if [ "$format" = deb ]; then
    command -v apt-get >/dev/null 2>&1 || fail "the .deb needs apt-get; set TRAYCI_FORMAT=appimage"
    if [ "$(id -u)" -eq 0 ]; then sudo=
    elif command -v sudo >/dev/null 2>&1; then sudo=sudo
    else fail "the .deb needs root or sudo; set TRAYCI_FORMAT=appimage"; fi
  fi

  step "Finding your release..."
  staging="$(mktemp -d)"
  download_pid=
  trap '[ -z "$download_pid" ] || { kill "$download_pid" 2>/dev/null || true; wait "$download_pid" 2>/dev/null || true; }; rm -rf "$staging"' EXIT
  trap 'printf "\n" >&2; exit 130' INT
  trap 'printf "\n" >&2; exit 143' TERM

  release_url="https://api.github.com/repos/${repo}/releases/latest"
  if [ -n "${TRAYCI_VERSION:-}" ]; then
    release_url="https://api.github.com/repos/${repo}/releases/tags/v${TRAYCI_VERSION#v}"
  fi
  fetch_status=0
  fetch "$release_url" "${staging}/release.json" || fetch_status=$?
  if [ "$fetch_status" -eq 44 ]; then
    fail "there is no Trayci release v${TRAYCI_VERSION#v}"
  elif [ "$fetch_status" -ne 0 ]; then
    fail "could not look up the release on GitHub"
  fi

  # The API answers pretty-printed JSON: one key per line, and inside an asset
  # its "name" comes before its "digest", which comes before its download URL.
  tag="$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "${staging}/release.json" | head -n 1)"
  [ -n "$tag" ] || fail "could not read the release from GitHub"
  version="${tag#v}"
  asset="Trayci_${version}_${suffix}"
  grep -q "\"name\": *\"${asset}\"" "${staging}/release.json" || fail "Trayci ${version} has no ${asset}"
  expected="$(awk -v name="\"${asset}\"" '
    index($0, "\"name\"") && index($0, name) { found = 1 }
    found && /"digest"/ { print; exit }
    found && /"browser_download_url"/ { exit }
  ' "${staging}/release.json" | sed -n 's/.*"sha256:\([0-9a-f]\{64\}\)".*/\1/p')"
  [ -n "$expected" ] || fail "GitHub has no SHA-256 digest for ${asset}, so it cannot be verified"

  if [ "$format" = deb ] && [ "$(dpkg-query -W -f='${Version}' trayci 2>/dev/null || true)" = "$version" ]; then
    if "$interactive"; then printf '\r\033[2K' >&2; fi
    printf '  %sTrayci %s is already installed.%s\n\n' "$green" "$version" "$reset" >&2
    return
  fi

  if "$interactive"; then printf '\r\033[2K' >&2; fi
  printf '  %sInstalling%s Trayci %s%s%s %s(%s)%s\n\n' "$muted" "$reset" "$bold" "$version" "$reset" "$muted" "$format" "$reset" >&2
  step "Downloading..."
  download "https://github.com/${repo}/releases/download/${tag}/${asset}" "${staging}/${asset}"

  step "Verifying the download..."
  [ "$(checksum "${staging}/${asset}")" = "$expected" ] || fail "checksum mismatch for ${asset}"

  if [ "$format" = deb ]; then
    step "Installing the package (sudo may ask for your password)..."
    if "$interactive"; then printf '\n' >&2; fi
    # apt reads local packages as its _apt user, which can't enter a 0700 mktemp dir.
    chmod 755 "$staging"; chmod 644 "${staging}/${asset}"
    # stdin is this script under `curl | sh`; apt must not read the rest of it.
    $sudo apt-get install -y "${staging}/${asset}" </dev/null || fail "apt-get could not install ${asset}"
    printf '\n  %sInstalled Trayci %s%s\n\n' "$green" "$version" "$reset" >&2
    printf '  Open %sTrayci%s from your app menu, or run %strayci &%s.\n\n' "$bold" "$reset" "$bold" "$reset"
    return
  fi

  bin_dir="${TRAYCI_INSTALL_DIR:-$HOME/.local/bin}"
  step "Setting up the trayci command..."
  mkdir -p "$bin_dir"
  chmod 755 "${staging}/${asset}"
  mv -f "${staging}/${asset}" "${bin_dir}/trayci"
  if "$interactive"; then printf '\r\033[2K' >&2; fi
  printf '  %sInstalled Trayci %s%s\n\n' "$green" "$version" "$reset" >&2
  if ! ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
    printf '  %sAppImages need libfuse2, which is missing. On Ubuntu 24.04: sudo apt install libfuse2t64%s\n' "$yellow" "$reset" >&2
  fi
  if [ -x /usr/bin/trayci ]; then
    printf '  %sThe .deb is installed too, at /usr/bin/trayci. Remove one so only one runs.%s\n' "$yellow" "$reset" >&2
  fi
  case ":${PATH}:" in
    *":${bin_dir}:"*) printf '  Run %strayci &%s to put it in your tray.\n\n' "$bold" "$reset" ;;
    *) printf '  Add %s to your PATH, then run %strayci &%s.\n\n' "$bin_dir" "$bold" "$reset" ;;
  esac
}

main "$@"
