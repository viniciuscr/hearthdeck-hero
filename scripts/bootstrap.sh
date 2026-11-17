#!/usr/bin/env bash
# Hearthdeck Hero bootstrap: turns a fresh Arch-based machine into the TV box.
#
#   curl -fsSL https://raw.githubusercontent.com/viniciuscr/hearthdeck-hero/fork/main/scripts/bootstrap.sh | sudo bash -s --
#
# What it does, in order: creates the console account, installs the packages
# (RetroArch and cores, Stremio, mpv, Dolphin, PCSX2, rclone, luakit,
# Ludusavi), installs Hearthdeck Hero itself, sets up RetroArch (controller
# profiles and screen settings), picks the sound card profile, mounts external
# disks, and makes tty1 boot straight into `cage` + `hearthdeck-hero --console`.
# At the end it can take the admin rights away from the console account.
#
# Safe to run again: every step reads the current state first, only changes
# what is missing or different, and backs up each file it edits as
# <file>.bak.<timestamp>. `--dry-run` prints the whole plan and changes nothing.
#
# Written for bash 5 on Arch, CachyOS and their relatives. See
# docs/BOOTSTRAP.md for the step-by-step reference and how to undo each part.
#
# Set HD_BOOTSTRAP_LIB=1 and `source` this file to use the helpers in tests;
# only then is `main` skipped.

set -euo pipefail

readonly HD_BOOTSTRAP_VERSION="1.0.0"
readonly HD_APP_NAME="hearthdeck-hero"
readonly HD_APP_USER_DEFAULT="hearthdeck-hero"
readonly HD_APP_BIN="/usr/bin/hearthdeck-hero"
readonly HD_REPO_NAME="hearthdeck"
readonly HD_REPO_URL_DEFAULT="https://viniciuscr.github.io/hearthdeck-hero/arch/x86_64"
readonly HD_RELEASES="https://github.com/viniciuscr/hearthdeck-hero/releases"
readonly HD_AUTOCONFIG_TARBALL="https://github.com/libretro/retroarch-joypad-autoconfig/archive/refs/heads/master.tar.gz"
readonly HD_MARK_BEGIN="# >>> hearthdeck-hero kiosk >>>"
readonly HD_MARK_END="# <<< hearthdeck-hero kiosk <<<"
readonly HD_MPV_BEGIN="# >>> hearthdeck-hero gamepad >>>"
readonly HD_MPV_END="# <<< hearthdeck-hero gamepad <<<"
# Overridable so a test harness can point it at a temporary directory.
HD_SUDO_DROPIN="${HD_SUDO_DROPIN:-/etc/sudoers.d/99-hearthdeck-bootstrap}"

# `pacman -S` argument lists. Every name is in Arch's `extra` (or CachyOS,
# which mirrors it); the AUR ones are handled separately.
readonly -a HD_PKGS_SESSION=(
  cage xorg-xwayland dbus polkit xdg-desktop-portal xdg-desktop-portal-gtk
  pipewire pipewire-alsa pipewire-pulse wireplumber ttf-dejavu noto-fonts
)
readonly -a HD_PKGS_EMULATION=(
  retroarch libretro-core-info libretro-shaders-slang
  libretro-snes9x libretro-mesen libretro-mgba libretro-genesis-plus-gx
  libretro-mupen64plus-next libretro-beetle-psx-hw libretro-melonds
  libretro-mame dolphin-emu
)
readonly -a HD_PKGS_MEDIA=(mpv rclone luakit)
readonly -a HD_PKGS_AUR=(stremio-service-bin pcsx2-latest-bin ludusavi)
readonly -a HD_DISPLAY_MANAGERS=(sddm gdm lightdm cosmic-greeter greetd ly lxdm slim xdm)

# Empty unless a terminal is available for questions; the script is usually
# piped into `sudo bash`, so stdin is the script, not the keyboard.
DRY_RUN=0
ASSUME_YES=0
REFRESH_AUTOCONFIG=0
NO_UPDATE=0
HD_APP_USER="$HD_APP_USER_DEFAULT"
CONTROLLER="both"
APP_PACKAGE=""
REPO_URL="$HD_REPO_URL_DEFAULT"
# The folder the app keeps its state in (`~/.config/<this>`); the TMDB step
# writes its key there.
HD_DATA_DIR="${HD_DATA_DIR:-hearthdeck}"
AUDIO_CARD=""
AUDIO_PROFILE=""
LOCKDOWN=0
LOG_FILE=""
LOGGING=0
SYSTEMD=0
STEP_INDEX=0
STEP_TOTAL=0
HD_WROTE=0
RUN_STAMP=""
# Overridable so the tests can point at a downloaded binary.
LUDUSAVI_BIN="${LUDUSAVI_BIN:-ludusavi}"
# Overridable so the tests can serve mock manifests from a file:// URL.
HD_MANIFESTS_URL="${HD_MANIFESTS_URL:-https://raw.githubusercontent.com/sharkusmanch/ludusavi-emulator-manifests/main/manifests}"
WORK_DIR=""
AUR_CACHE=""
BOOTSTRAP_SUDO_FILE=""
MOUNTS=()
SKIP=()
ONLY=()
SELECTED=()
CHANGES=()
# Paths a dry run has already announced, so two settings in one file read as
# one "would write" line.
declare -A HD_DRY_REPORTED=()

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

if [[ -t 1 && -z ${NO_COLOR:-} ]]; then
  C_BOLD=$'\e[1m'; C_DIM=$'\e[2m'; C_GREEN=$'\e[32m'
  C_YELLOW=$'\e[33m'; C_RED=$'\e[31m'; C_RESET=$'\e[0m'
else
  C_BOLD=""; C_DIM=""; C_GREEN=""; C_YELLOW=""; C_RED=""; C_RESET=""
fi

log_line() {
  [[ $LOGGING -eq 1 ]] || return 0
  # Never let a full or read-only log decide the outcome of the run.
  printf '%s\n' "$*" >>"$LOG_FILE" 2>/dev/null || true
}

say() {
  printf '%s\n' "$*"
  log_line "$*"
}

info() {
  printf '  %s\n' "$*"
  log_line "  $*"
}

ok() {
  printf '  %s✓%s %s\n' "$C_GREEN" "$C_RESET" "$*"
  log_line "  ok: $*"
}

warn() {
  printf '  %s!%s %s\n' "$C_YELLOW" "$C_RESET" "$*" >&2
  log_line "  warning: $*"
}

die() {
  printf '%s%sbootstrap: %s%s\n' "$C_BOLD" "$C_RED" "$*" "$C_RESET" >&2
  log_line "  fatal: $*"
  exit 1
}

note() {
  CHANGES+=("$*")
}

section() {
  STEP_INDEX=$((STEP_INDEX + 1))
  printf '\n%s[%d/%d] %s%s\n' "$C_BOLD" "$STEP_INDEX" "$STEP_TOTAL" "$1" "$C_RESET"
  log_line ""
  log_line "[$STEP_INDEX/$STEP_TOTAL] $1"
}

banner() {
  printf '%s\n' "${C_BOLD}Hearthdeck Hero bootstrap ${HD_BOOTSTRAP_VERSION}${C_RESET}"
  say "Arch-based TV console setup: packages, RetroArch, Stremio, emulators,"
  say "external disks and a tty1 that boots into Hearthdeck Hero."
  if [[ $DRY_RUN -eq 1 ]]; then
    say "${C_YELLOW}Dry run: nothing will be changed, and nothing will be asked.${C_RESET}"
  fi
}

# ---------------------------------------------------------------------------
# Questions
# ---------------------------------------------------------------------------

# True when questions can reach the person even though stdin is the script:
# /dev/tty must be openable, not merely present.
tty_available() {
  { : </dev/tty; } 2>/dev/null
}

# Sets REPLY. Only --yes answers for a machine with no terminal: otherwise the
# question is left unanswered, so a risky default is never taken silently.
prompt() {
  local question="$1" default="${2-}" answer=""
  REPLY=""
  if [[ $DRY_RUN -eq 1 ]]; then
    REPLY="$default"
    return 0
  fi
  if [[ $ASSUME_YES -eq 1 ]]; then
    REPLY="$default"
    return 0
  fi
  if ! tty_available; then
    warn "no terminal to ask '$question': leaving it alone (pass -y to accept defaults)"
    return 1
  fi
  printf '%s' "$question" >/dev/tty || return 1
  [[ -n $default ]] && { printf ' [%s]' "$default" >/dev/tty || return 1; }
  printf ': ' >/dev/tty || return 1
  IFS= read -r answer </dev/tty || return 1
  REPLY="${answer:-$default}"
}

# confirm QUESTION [DEFAULT] — true for y/yes.
confirm() {
  local question="$1" default="${2:-y}"
  prompt "$question" "$default" || return 1
  case "${REPLY,,}" in
    y | yes) return 0 ;;
    *) return 1 ;;
  esac
}

# choose QUESTION COUNT — sets REPLY to a number between 1 and COUNT, with 0
# meaning "leave it alone". Returns non-zero when nobody can answer.
choose_number() {
  local question="$1" count="$2" value
  if [[ $DRY_RUN -eq 1 ]] || [[ $ASSUME_YES -eq 1 ]] || ! tty_available; then
    warn "no way to ask '$question' here: leaving it alone"
    return 1
  fi
  while :; do
    if ! prompt "$question" ""; then return 1; fi
    value="$REPLY"
    if [[ $value =~ ^[0-9]+$ ]] && ((value >= 0 && value <= count)); then
      REPLY="$value"
      return 0
    fi
    info "enter a number between 0 and $count"
  done
}

# ---------------------------------------------------------------------------
# Running commands
# ---------------------------------------------------------------------------

# Prints and runs. In a dry run it only prints.
run() {
  if [[ $DRY_RUN -eq 1 ]]; then
    printf '  %s[dry-run] %s%s\n' "$C_DIM" "$*" "$C_RESET"
    log_line "  [dry-run] $*"
    return 0
  fi
  printf '  %s$ %s%s\n' "$C_DIM" "$*" "$C_RESET"
  log_line "  $ $*"
  "$@"
}

# Writes stdin to PATH when the content differs from what is on disk. Sets
# HD_WROTE=1 when it wrote. BACKUP first, then owned and chmodded.
write_if_changed() {
  local path="$1" owner="${2:-root:root}" mode="${3:-0644}" tmp
  tmp="$WORK_DIR/new.$$.$RANDOM"
  cat >"$tmp"
  commit_file "$path" "$tmp" "$owner" "$mode"
}

backup_file() {
  local path="$1" backup
  [[ -e $path ]] || return 0
  backup="$path.bak.$RUN_STAMP"
  [[ -e $backup ]] && return 0
  cp -a -- "$path" "$backup"
  say "  ${C_DIM}backed up $path -> $(basename -- "$backup")${C_RESET}"
}

# commit_file PATH TMP OWNER MODE — installs TMP at PATH when it differs.
# Sets HD_WROTE=1 and creates PATH (with parents) when it does.
commit_file() {
  local path="$1" tmp="$2" owner="${3:-root:root}" mode="${4:-0644}"
  local dir target
  HD_WROTE=0
  if [[ -f $path ]] && cmp -s -- "$tmp" "$path"; then
    rm -f -- "$tmp"
    return 0
  fi
  if [[ $DRY_RUN -eq 1 ]]; then
    if [[ -z ${HD_DRY_REPORTED[$path]:-} ]]; then
      say "  ${C_DIM}[dry-run]${C_RESET} would write $path"
      HD_DRY_REPORTED[$path]=1
    fi
    rm -f -- "$tmp"
    HD_WROTE=1
    return 0
  fi
  backup_file "$path"
  dir=$(dirname -- "$path")
  mkdir -p -- "$dir"
  # Write beside the target, then rename: /etc/fstab and pacman.conf are never
  # left half-written, even if the disk fills up.
  target=$(mktemp "$dir/.hearthdeck-bootstrap.XXXXXX")
  cat -- "$tmp" >"$target"
  rm -f -- "$tmp"
  chmod -- "$mode" "$target"
  chown -- "$owner" "$target"
  mv -f -- "$target" "$path"
  HD_WROTE=1
}

# ensure_block FILE BEGIN END OWNER — replaces the region between BEGIN and
# END (inclusive) with stdin, or appends it when there is none. Everything
# outside the region is kept as it was.
ensure_block() {
  local file="$1" begin="$2" end="$3" owner="${4:-root:root}"
  local block tmp has_begin=0 has_end=0
  block="$WORK_DIR/block.$$.$RANDOM"
  cat >"$block"
  tmp="$WORK_DIR/merged.$$.$RANDOM"
  if [[ -f $file ]]; then
    if grep -qxF -- "$begin" "$file"; then has_begin=1; fi
    if grep -qxF -- "$end" "$file"; then has_end=1; fi
    if [[ $has_begin -ne $has_end ]]; then
      warn "$file has only one of the bootstrap's markers; leaving it alone"
      rm -f -- "$block"
      return 1
    fi
  fi
  if [[ $has_begin -eq 1 ]]; then
    awk -v block="$block" -v begin="$begin" -v end="$end" '
      BEGIN { placed = 0 }
      $0 == begin {
        if (!placed) {
          while ((getline line < block) > 0) print line
          close(block)
          placed = 1
        }
        skipping = 1
        next
      }
      skipping && $0 == end { skipping = 0; next }
      !skipping { print }
      END {
        if (!placed) {
          while ((getline line < block) > 0) print line
          close(block)
        }
      }
    ' "$file" >"$tmp"
  elif [[ -f $file && -s $file ]]; then
    cat -- "$file" >"$tmp"
    # One blank line between the existing file and the managed block, even
    # when the file does not end with a newline.
    [[ -n $(tail -c 1 -- "$tmp") ]] && printf '\n' >>"$tmp"
    printf '\n' >>"$tmp"
    cat -- "$block" >>"$tmp"
  else
    cp -- "$block" "$tmp"
  fi
  rm -f -- "$block"
  commit_file "$file" "$tmp" "$owner" "0644"
}

# set_kv FILE KEY VALUE OWNER — sets `KEY = VALUE` in a RetroArch-style config,
# keeping every other line. Creates the file with a short header if absent.
set_kv() {
  local file="$1" key="$2" value="$3" owner="${4:-root:root}"
  local tmp line found=0
  tmp="$WORK_DIR/kv.$$.$RANDOM"
  if [[ -f $file ]]; then
    while IFS= read -r line || [[ -n $line ]]; do
      if [[ $line =~ ^[[:space:]]*"$key"[[:space:]]*= ]]; then
        printf '%s = %s\n' "$key" "$value"
        found=1
      else
        printf '%s\n' "$line"
      fi
    done <"$file" >"$tmp"
  else
    : >"$tmp"
    printf '# Written by the Hearthdeck Hero bootstrap.\n' >>"$tmp"
    printf '# RetroArch fills in its remaining settings on the first start.\n' >>"$tmp"
  fi
  if [[ $found -eq 0 ]]; then
    printf '%s = %s\n' "$key" "$value" >>"$tmp"
  fi
  commit_file "$file" "$tmp" "$owner" "0644"
}

# ---------------------------------------------------------------------------
# System helpers
# ---------------------------------------------------------------------------

home_of() {
  getent passwd "$1" 2>/dev/null | cut -d: -f6
}

# The default is only for a dry run, where the account does not exist yet.
home_or_default() {
  local home
  home=$(home_of "$1" || true)
  printf '%s\n' "${home:-/home/$1}"
}

user_uid() {
  id -u "$1" 2>/dev/null || printf '1000\n'
}

user_gid() {
  id -g "$1" 2>/dev/null || printf '1000\n'
}

user_in_group() {
  id -nG "$1" 2>/dev/null | tr ' ' '\n' | grep -qx -- "$2"
}

user_has_password() {
  [[ $(passwd -S "$1" 2>/dev/null | awk '{print $2}') == "P" ]]
}

# as_user USER CMD... — runs a command as another account, with that account's
# home and (when present) its running session environment.
as_user() {
  local user="$1"; shift
  local home uid
  home=$(home_or_default "$user")
  uid=$(user_uid "$user")
  local -a envs=(HOME="$home" USER="$user" LOGNAME="$user")
  if [[ -d /run/user/$uid ]]; then
    envs+=(XDG_RUNTIME_DIR="/run/user/$uid")
    [[ -S /run/user/$uid/bus ]] && envs+=(DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus")
  fi
  if [[ $EUID -ne 0 ]]; then
    # Only the tests (HD_BOOTSTRAP_LIB) get here; preflight insists on root.
    run env "${envs[@]}" "$@"
    return
  fi
  run runuser -u "$user" -- env "${envs[@]}" "$@"
}

# pactl_as_user USER ARGS... — talks to the account's PipeWire, if it is up.
pactl_as_user() {
  local user="$1"; shift
  local uid
  uid=$(user_uid "$user")
  runuser -u "$user" -- env \
    XDG_RUNTIME_DIR="/run/user/$uid" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" \
    pactl "$@"
}

user_audio_session() {
  local uid
  uid=$(id -u "$1" 2>/dev/null) || return 1
  [[ -S /run/user/$uid/bus && -S /run/user/$uid/pipewire-0 ]] || return 1
  pactl_as_user "$1" info >/dev/null 2>&1
}

# pacman_install PKG... — installs only the packages that are missing.
pacman_install() {
  local -a missing=() pkgs=("$@") p
  for p in "${pkgs[@]}"; do
    pacman -Q "$p" >/dev/null 2>&1 || missing+=("$p")
  done
  if [[ ${#missing[@]} -eq 0 ]]; then
    ok "already installed: ${pkgs[*]}"
    return 0
  fi
  info "installing: ${missing[*]}"
  if ! run pacman -S --needed --noconfirm "${missing[@]}"; then
    die "pacman could not install ${missing[*]}; on a dependency error run 'pacman -Syu' first (or drop --no-update)"
  fi
  note "installed ${#missing[@]} package(s): ${missing[*]}"
}

# The AUR helper the machine already has, if any.
aur_helper() {
  local helper
  for helper in shelly paru yay; do
    if command -v "$helper" >/dev/null 2>&1; then
      printf '%s\n' "$helper"
      return 0
    fi
  done
  return 1
}

# Lets the console account build AUR packages without a password prompt while
# the bootstrap runs. Removed by `cleanup` even when a step fails.
install_bootstrap_sudo() {
  local user="$1"
  if [[ $DRY_RUN -eq 1 ]]; then
    say "  ${C_DIM}[dry-run]${C_RESET} would let $user run sudo without a password during setup"
    return 0
  fi
  if [[ -e $HD_SUDO_DROPIN ]]; then
    die "$HD_SUDO_DROPIN is left over from an interrupted run; remove it with 'rm $HD_SUDO_DROPIN' and run again"
  fi
  if ! command -v visudo >/dev/null 2>&1; then
    warn "visudo is missing: AUR builds may ask $user for a password"
    return 0
  fi
  {
    printf '# Created by the Hearthdeck Hero bootstrap; removed when it finishes.\n'
    printf '%s ALL=(ALL) NOPASSWD: ALL\n' "$user"
  } >"$HD_SUDO_DROPIN"
  chmod 0440 "$HD_SUDO_DROPIN"
  if ! visudo -cf "$HD_SUDO_DROPIN" >/dev/null 2>&1; then
    rm -f "$HD_SUDO_DROPIN"
    warn "could not install a temporary sudo rule; AUR builds may ask for a password"
    return 0
  fi
  BOOTSTRAP_SUDO_FILE="$HD_SUDO_DROPIN"
}

# The display managers that would fight the console for the screen.
enabled_display_managers() {
  local dm
  for dm in "${HD_DISPLAY_MANAGERS[@]}"; do
    if systemctl is-enabled --quiet "$dm.service" 2>/dev/null; then
      printf '%s\n' "$dm"
    fi
  done
}

# Every account in the wheel group other than SELF.
other_wheel_members() {
  local self="$1" member
  getent group wheel 2>/dev/null | cut -d: -f4 | tr ',' '\n' |
    while IFS= read -r member; do
      [[ -n $member && $member != "$self" ]] && printf '%s\n' "$member"
    done
}

wheel_can_sudo() {
  local snippet
  for snippet in /etc/sudoers /etc/sudoers.d/*; do
    [[ -r $snippet ]] || continue
    if grep -Eq '^[[:space:]]*%wheel[[:space:]]+[^#]*ALL' "$snippet"; then
      return 0
    fi
  done
  return 1
}

fstab_has_uuid() {
  local wanted="$1" file="${2:-/etc/fstab}"
  [[ -r $file ]] || return 1
  awk -v u="UUID=$wanted" '$1 == u { found = 1 } END { exit !found }' "$file"
}

# The line an external filesystem gets in /etc/fstab.
fstab_line() {
  local uuid="$1" path="$2" fstype="$3" uid="$4" gid="$5"
  printf 'UUID=%s  %s  %s  %s  0  0' "$uuid" "$path" "$fstype" \
    "$(external_fs_options "$fstype" "$uid" "$gid")"
}

# parse_mount_spec SPEC — prints "<uuid>\t<path>" or returns 1. Accepts
# UUID=<uuid>=/path and <uuid>=/path, since a bare UUID is easier to paste.
parse_mount_spec() {
  local spec="$1" uuid path
  spec="${spec#UUID=}"
  uuid="${spec%%=*}"
  path="${spec#*=}"
  if [[ -z $uuid || -z $path || $path != /* || $path == *[[:space:]]* ]]; then
    return 1
  fi
  printf '%s\t%s\n' "$uuid" "$path"
}

# The options an external disk gets: usable by the console account, and
# mounted on demand so an unplugged disk never blocks the boot.
external_fs_options() {
  local fstype="$1" uid="$2" gid="$3"
  case "$fstype" in
    exfat | vfat | ntfs | ntfs3 | fuseblk)
      printf 'defaults,noatime,uid=%s,gid=%s,nofail,x-systemd.automount,x-systemd.device-timeout=5' "$uid" "$gid"
      ;;
    *)
      printf 'defaults,noatime,nofail,x-systemd.automount,x-systemd.device-timeout=5'
      ;;
  esac
}

# Picks a GPU driver package set from the DRM devices, if they are readable.
gpu_packages() {
  local file vendor
  local -a found=()
  for file in /sys/class/drm/card*/device/vendor; do
    [[ -r $file ]] || continue
    vendor=$(<"$file")
    case "$vendor" in
      0x1002) found+=(vulkan-radeon libva-mesa-driver) ;;
      0x8086) found+=(vulkan-intel intel-media-driver) ;;
      0x10de) found+=(nvidia-utils) ;;
    esac
  done
  [[ ${#found[@]} -gt 0 ]] || return 1
  printf '%s\n' "${found[@]}" | sort -u
}

has_nvidia_driver() {
  pacman -Q nvidia nvidia-open nvidia-open-dkms nvidia-dkms nvidia-lts >/dev/null 2>&1 ||
    [[ -d /sys/module/nvidia ]]
}

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------

cleanup() {
  local rc=$?
  trap - EXIT
  if [[ -n $BOOTSTRAP_SUDO_FILE && -e $BOOTSTRAP_SUDO_FILE ]]; then
    rm -f "$BOOTSTRAP_SUDO_FILE"
    say "removed the temporary sudo rule for the setup"
  fi
  if [[ -n $AUR_CACHE && $AUR_CACHE == */.cache/hearthdeck-bootstrap/* && -d $AUR_CACHE ]]; then
    rm -rf -- "$AUR_CACHE"
  fi
  if [[ -n $WORK_DIR && -d $WORK_DIR ]]; then
    rm -rf -- "$WORK_DIR"
  fi
  exit $rc
}

# ---------------------------------------------------------------------------
# Steps
# ---------------------------------------------------------------------------

step_user() {
  section "Console account"
  local user="$HD_APP_USER" home group
  local -a groups=()

  if id "$user" >/dev/null 2>&1; then
    ok "the account $user already exists"
  else
    for group in video audio input render; do
      getent group "$group" >/dev/null 2>&1 && groups+=("$group")
    done
    info "creating $user (the login password stays locked; autologin needs none)"
    local -a group_args=()
    [[ ${#groups[@]} -gt 0 ]] && group_args=(--groups "$(IFS=,; printf '%s' "${groups[*]}")")
    run useradd --create-home --shell /bin/bash "${group_args[@]}" "$user"
    note "created the account $user"
  fi

  home=$(home_or_default "$user")
  if [[ ! -d $home ]]; then
    run install -d -o "$user" -g "$user" -m 0755 "$home"
  fi

  # No wheel here: the AUR step borrows sudo through a rule that is removed
  # when the bootstrap exits, so the account never needs permanent admin.
  for group in video audio input render; do
    if ! getent group "$group" >/dev/null 2>&1; then
      warn "the group $group does not exist on this system; skipped"
      continue
    fi
    if user_in_group "$user" "$group"; then
      continue
    fi
    run usermod --append --groups "$group" "$user"
    note "added $user to the $group group"
  done

  if [[ $(getent passwd "$user" | cut -d: -f7) != "/bin/bash" ]]; then
    run usermod --shell /bin/bash "$user"
    note "set $user's shell to /bin/bash"
  fi

  if ! user_has_password "$user"; then
    if confirm "Set a password for $user now (SSH and sudo during setup use it)?" y; then
      if [[ $DRY_RUN -eq 1 ]]; then
        say "  ${C_DIM}[dry-run]${C_RESET} would run passwd $user"
      else
        passwd "$user"
      fi
      note "set a password for $user"
    else
      info "no password: autologin still works, and SSH needs a key"
    fi
  fi

  if [[ $user != "$HD_APP_USER_DEFAULT" ]]; then
    info "note: the kiosk profile and every path below use $user"
  fi
}

step_packages() {
  section "Packages"
  info "session, emulators, media and backup tools from the official repositories"

  if [[ $NO_UPDATE -eq 0 ]] && confirm "Update the system first (pacman -Syu)?" y; then
    local -a pending=()
    mapfile -t pending < <(pacman -Qu 2>/dev/null || true)
    run pacman -Syu --noconfirm
    if [[ ${#pending[@]} -gt 0 ]]; then
      note "updated ${#pending[@]} package(s) with pacman -Syu"
    fi
  else
    warn "skipping the system update: installing from an old database can conflict"
  fi

  pacman_install git curl vulkan-icd-loader
  if ! pacman -Qg base-devel >/dev/null 2>&1; then
    run pacman -S --needed --noconfirm base-devel
    note "installed the base-devel group (build tools)"
  fi
  pacman_install "${HD_PKGS_SESSION[@]}"
  pacman_install "${HD_PKGS_EMULATION[@]}"
  pacman_install "${HD_PKGS_MEDIA[@]}"

  local -a gpu=() usable=()
  if mapfile -t gpu < <(gpu_packages) && [[ ${#gpu[@]} -gt 0 ]]; then
    if [[ " ${gpu[*]} " == *" nvidia-utils "* ]] && ! has_nvidia_driver; then
      warn "NVIDIA GPU found but no NVIDIA kernel driver: install nvidia or nvidia-open for your kernel"
      warn "then re-run the bootstrap; the Vulkan loader is installed, the driver is not"
    fi
    local candidate
    for candidate in "${gpu[@]}"; do
      [[ -n $candidate && $candidate != "nvidia-utils" ]] && usable+=("$candidate")
    done
    if [[ ${#usable[@]} -gt 0 ]]; then
      pacman_install "${usable[@]}"
    fi
  else
    warn "could not read the GPU vendor from /sys/class/drm; install the Vulkan driver by hand"
  fi
}

step_aur() {
  section "AUR packages"
  local user="$HD_APP_USER" helper="" pkg
  local -a missing=()

  for pkg in "${HD_PKGS_AUR[@]}"; do
    pacman -Q "$pkg" >/dev/null 2>&1 || missing+=("$pkg")
  done
  if [[ ${#missing[@]} -eq 0 ]]; then
    ok "already installed: ${HD_PKGS_AUR[*]}"
    return 0
  fi
  info "from the AUR: ${missing[*]}"

  pacman_install git
  if ! pacman -Qg base-devel >/dev/null 2>&1; then
    run pacman -S --needed --noconfirm base-devel
    note "installed the base-devel group (build tools)"
  fi
  install_bootstrap_sudo "$user"

  if helper=$(aur_helper); then
    ok "using $helper"
  else
    warn "no AUR helper (shelly, paru, yay): building with makepkg as $user"
  fi

  if [[ $DRY_RUN -eq 1 ]]; then
    say "  ${C_DIM}[dry-run]${C_RESET} would install ${missing[*]} as $user"
    return 0
  fi

  local installed=1
  if [[ -n $helper ]]; then
    case "$helper" in
      shelly) as_user "$user" shelly -n install aur "${missing[@]}" || installed=0 ;;
      paru) as_user "$user" paru -S --needed --noconfirm "${missing[@]}" || installed=0 ;;
      yay) as_user "$user" yay -S --needed --noconfirm "${missing[@]}" || installed=0 ;;
    esac
    [[ $installed -eq 0 ]] && warn "$helper could not install everything; trying makepkg"
  fi
  if [[ -z $helper || $installed -eq 0 ]]; then
    build_with_makepkg "$user" "${missing[@]}"
  fi

  local -a still_missing=()
  for pkg in "${missing[@]}"; do
    pacman -Q "$pkg" >/dev/null 2>&1 && note "installed $pkg from the AUR" || still_missing+=("$pkg")
  done
  if [[ ${#still_missing[@]} -gt 0 ]]; then
    warn "not installed: ${still_missing[*]} (install them by hand and re-run)"
  fi
}

# build_with_makepkg USER PKG... — the fallback when no AUR helper exists.
build_with_makepkg() {
  local user="$1"; shift
  local home cache pkg dir failed=0
  home=$(home_or_default "$user")
  cache="$home/.cache/hearthdeck-bootstrap/aur"
  AUR_CACHE="$cache"
  run install -d -o "$user" -g "$user" -m 0755 "$cache"

  for pkg in "$@"; do
    dir="$cache/$pkg"
    if as_user "$user" git clone --depth 1 "https://aur.archlinux.org/$pkg.git" "$dir"; then
      if ! (cd "$dir" && as_user "$user" makepkg -si --noconfirm); then
        warn "makepkg failed for $pkg"
        failed=1
      fi
    else
      warn "could not clone the AUR repository for $pkg"
      failed=1
    fi
  done
  [[ $failed -eq 0 ]] || warn "some AUR packages need a manual build; see $cache"
}

# ---------------------------------------------------------------------------
# Ludusavi: per-game emulator saves
# ---------------------------------------------------------------------------

# yaml_quote_path PATH — double-quotes a path for YAML when it needs it.
yaml_quote_path() {
  case "$1" in
    *[!A-Za-z0-9/._+-]*) printf '"%s"' "${1//\"/\\\"}" ;;
    *) printf '%s' "$1" ;;
  esac
}

# yaml_line_value LINE — the text after the first colon, without indentation.
yaml_line_value() {
  local value="${1#*:}"
  printf '%s' "${value#"${value%%[![:space:]]*}"}"
}

# yaml_has_list_item FILE PATH — true when FILE already lists that path,
# quoted or not. Ludusavi writes a root as `- path: …` the first time and as
# `- store: …` followed by `path: …` after it rewrites the file, so a `path:`
# line anywhere counts.
yaml_has_list_item() {
  local file="$1" want="$2"
  [[ -f $file ]] || return 1
  awk -v want="$want" -v q="'" '
    /^[[:space:]]*(-[[:space:]]*)?path:[[:space:]]*/ {
      line = $0
      sub(/^[[:space:]]*(-[[:space:]]*)?path:[[:space:]]*/, "", line)
      sub(/[[:space:]]+$/, "", line)
      gsub("[\"" q "]", "", line)
      if (line == want) found = 1
    }
    END { exit !found }
  ' "$file"
}

# yaml_list_ensure FILE PARENT KEY KEY_INDENT ITEMS_FILE OWNER
#   Adds the pre-indented ITEMS_FILE lines under KEY, keeping every other line
#   as it was. PARENT empty means a top-level key; otherwise KEY sits at
#   KEY_INDENT spaces inside PARENT. Returns 0 after writing (or, in a dry run,
#   reporting), 1 when there was nothing to add, 3 when the YAML shape is not
#   one this understands — in which case nothing is written.
yaml_list_ensure() {
  local file="$1" parent="$2" key="$3" kindent="$4" items="$5" owner="$6"
  local -a lines=() out=() add=()
  local i line key_line=-1 parent_line=-1 in_parent=0 value key_pad n
  [[ -f $file ]] && mapfile -t lines <"$file"
  mapfile -t add <"$items"
  [[ ${#add[@]} -gt 0 ]] || return 1
  printf -v key_pad '%*s' "$kindent" ''

  n=${#lines[@]}
  for ((i = 0; i < n; i++)); do
    line="${lines[i]}"
    if [[ -z $parent ]]; then
      if [[ $line == "${key_pad}${key}:" || $line == "${key_pad}${key}: "* ]]; then
        key_line=$i
        break
      fi
      continue
    fi
    if ((in_parent == 0)); then
      if [[ $line == "${parent}:" || $line == "${parent}: "* ]]; then
        in_parent=1
        parent_line=$i
      fi
      continue
    fi
    if [[ $line == "${key_pad}${key}:" || $line == "${key_pad}${key}: "* ]]; then
      key_line=$i
      break
    fi
    if [[ -n ${line//[[:space:]]/} && ${line:0:1} != ' ' && ${line:0:1} != '#' ]]; then
      in_parent=0
    fi
  done

  if ((key_line >= 0)); then
    value=$(yaml_line_value "${lines[key_line]}")
    case "$value" in
      "[]")
        out=("${lines[@]:0:key_line}")
        out+=("${key_pad}${key}:")
        out+=("${add[@]}")
        out+=("${lines[@]:key_line+1}")
        ;;
      \[*\])
        return 3
        ;;
      *)
        # A block list (or a key with no value yet): prepend the new items so
        # the existing ones keep their order.
        out=("${lines[@]:0:key_line+1}")
        out+=("${add[@]}")
        out+=("${lines[@]:key_line+1}")
        ;;
    esac
  elif [[ -z $parent ]]; then
    out=("${lines[@]}")
    out+=("${key_pad}${key}:")
    out+=("${add[@]}")
  elif ((parent_line >= 0)); then
    value=$(yaml_line_value "${lines[parent_line]}")
    case "$value" in
      "{}")
        out=("${lines[@]:0:parent_line}")
        out+=("${parent}:")
        out+=("${key_pad}${key}:")
        out+=("${add[@]}")
        out+=("${lines[@]:parent_line+1}")
        ;;
      "")
        out=("${lines[@]:0:parent_line+1}")
        out+=("${key_pad}${key}:")
        out+=("${add[@]}")
        out+=("${lines[@]:parent_line+1}")
        ;;
      *)
        return 3
        ;;
    esac
  else
    out=("${lines[@]}")
    out+=("${parent}:")
    out+=("${key_pad}${key}:")
    out+=("${add[@]}")
  fi

  local tmp="$WORK_DIR/yaml.$$.$RANDOM"
  if [[ ${#out[@]} -gt 0 ]]; then
    printf '%s\n' "${out[@]}" >"$tmp"
  else
    : >"$tmp"
  fi
  commit_file "$file" "$tmp" "$owner" "0644"
  return 0
}

# emulator_installed PKG... — true when any of these packages is installed.
emulator_installed() {
  local package
  for package in "$@"; do
    if pacman -Q "$package" >/dev/null 2>&1; then
      return 0
    fi
  done
  return 1
}

# ludusavi_targets HOME — prints "<manifest>\t<root>" for each installed
# emulator, pointing at the folder its saves actually live in on Linux:
# RetroArch's config folder (saves/ and states/), Dolphin's XDG data folder,
# and PCSX2's folder memory cards (DataRoot = $XDG_CONFIG_HOME/PCSX2).
ludusavi_targets() {
  local home="$1"
  if emulator_installed retroarch; then
    printf 'retroarch\t%s\n' "$home/.config/retroarch"
  fi
  if emulator_installed dolphin-emu; then
    printf 'dolphin\t%s\n' "$home/.local/share/dolphin-emu"
  fi
  if emulator_installed pcsx2 pcsx2-latest-bin pcsx2-git pcsx2-avx2; then
    printf 'pcsx2\t%s\n' "$home/.config/PCSX2/memcards"
  fi
}

# ludusavi_setup USER CONFIG_DIR MANIFEST_DIR TARGETS_FILE
#   Downloads the per-emulator manifests, makes the folders they read, and
#   teaches Ludusavi about both. Ludusavi itself validates the result; if it
#   refuses, the previous config is put back.
ludusavi_setup() {
  local user="$1" cfgdir="$2" mandir="$3" targets="$4" owner="$1:$1"
  local cfg="$cfgdir/config.yaml" name root url file dl items rc=0
  local -a names=() roots=()
  local changed=0 refreshed=0 i

  while IFS=$'\t' read -r name root; do
    [[ -n $name ]] || continue
    names+=("$name")
    roots+=("$root")
  done <"$targets"
  [[ ${#names[@]} -gt 0 ]] || return 0

  for root in "${roots[@]}"; do
    run install -d -o "$user" -g "$user" -m 0755 "$root"
  done
  run install -d -o "$user" -g "$user" -m 0755 "$mandir"

  for i in "${!names[@]}"; do
    name="${names[i]}"
    url="$HD_MANIFESTS_URL/$name.yaml"
    file="$mandir/$name.yaml"
    dl="$WORK_DIR/manifest-$name.yaml"
    if ! curl -fsSL --max-time 300 -o "$dl" "$url"; then
      warn "could not download $url; keeping the manifest already in $file"
      continue
    fi
    if ! grep -q '^[[:space:]]*files:' "$dl" || ! grep -q '<root>' "$dl"; then
      warn "$url is not a Ludusavi manifest; keeping $file"
      continue
    fi
    write_if_changed "$file" "$owner" "0644" <"$dl"
    if [[ $HD_WROTE -eq 1 ]]; then
      refreshed=$((refreshed + 1))
    fi
  done

  items="$WORK_DIR/ludusavi-roots.items"
  : >"$items"
  for root in "${roots[@]}"; do
    if yaml_has_list_item "$cfg" "$root"; then continue; fi
    {
      printf '  - path: %s\n' "$(yaml_quote_path "$root")"
      printf '    store: other\n'
    } >>"$items"
  done
  if [[ -s $items ]]; then
    yaml_list_ensure "$cfg" "" "roots" 0 "$items" "$owner"
    rc=$?
    if [[ $rc -eq 3 ]]; then
      warn "$cfg has a roots: list this script does not understand; add these by hand:"
      sed 's/^/      /' "$items" >&2
    elif [[ $rc -eq 0 ]]; then
      changed=1
    fi
  fi

  items="$WORK_DIR/ludusavi-secondary.items"
  : >"$items"
  for name in "${names[@]}"; do
    file="$mandir/$name.yaml"
    if yaml_has_list_item "$cfg" "$file"; then continue; fi
    printf '    - path: %s\n' "$(yaml_quote_path "$file")" >>"$items"
  done
  if [[ -s $items ]]; then
    yaml_list_ensure "$cfg" "manifest" "secondary" 2 "$items" "$owner"
    rc=$?
    if [[ $rc -eq 3 ]]; then
      warn "$cfg has a manifest: block this script does not understand; add these by hand:"
      sed 's/^/      /' "$items" >&2
    elif [[ $rc -eq 0 ]]; then
      changed=1
    fi
  fi

  if [[ $refreshed -gt 0 ]]; then
    note "downloaded $refreshed Ludusavi emulator manifest(s)"
  fi
  if [[ $changed -eq 0 ]]; then
    ok "Ludusavi already knows these emulators"
    return 0
  fi

  if ! as_user "$user" "$LUDUSAVI_BIN" --config "$cfgdir" --no-manifest-update config show >/dev/null 2>&1; then
    warn "Ludusavi rejected the edited $cfg; putting the previous file back"
    if [[ -f "$cfg.bak.$RUN_STAMP" ]]; then
      cp -a -- "$cfg.bak.$RUN_STAMP" "$cfg"
    else
      rm -f -- "$cfg"
    fi
    note "Ludusavi config left alone (it could not be updated)"
    return 0
  fi
  note "Ludusavi: per-game saves for ${names[*]}"
}

step_ludusavi() {
  section "Ludusavi"
  local user="$HD_APP_USER" home cfgdir mandir targets name root
  home=$(home_or_default "$user")
  cfgdir="$home/.config/ludusavi"
  mandir="$cfgdir/manifests"
  targets="$WORK_DIR/ludusavi-targets"

  if ! command -v "$LUDUSAVI_BIN" >/dev/null 2>&1; then
    if [[ $DRY_RUN -eq 1 ]]; then
      info "Ludusavi comes from the AUR step; this step then teaches it the emulator saves"
      return 0
    fi
    warn "Ludusavi is not installed, so its emulator manifests wait"
    return 0
  fi
  ludusavi_targets "$home" >"$targets"
  if [[ ! -s $targets ]]; then
    if [[ $DRY_RUN -eq 1 ]]; then
      info "no emulator installed yet; after the packages step this adds RetroArch, Dolphin and PCSX2"
    else
      ok "no emulator this bootstrap knows is installed; nothing to add"
    fi
    return 0
  fi
  info "per-game emulator saves:"
  while IFS=$'\t' read -r name root; do
    printf '    %-10s %s\n' "$name" "$root"
  done <"$targets"

  if [[ $DRY_RUN -eq 1 ]]; then
    say "  ${C_DIM}[dry-run]${C_RESET} would download the manifests into $mandir and add them to $cfgdir/config.yaml"
    return 0
  fi
  run install -d -o "$user" -g "$user" -m 0755 "$cfgdir"
  ludusavi_setup "$user" "$cfgdir" "$mandir" "$targets"
}

step_app() {
  section "Hearthdeck Hero"
  local current=""

  if [[ -n $APP_PACKAGE ]]; then
    info "installing the package file $APP_PACKAGE"
    if [[ ! -f $APP_PACKAGE ]]; then
      die "no such file: $APP_PACKAGE"
    fi
    run pacman -U --needed --noconfirm "$APP_PACKAGE"
    note "installed Hearthdeck Hero from $APP_PACKAGE"
    return 0
  fi

  if pacman -Q "$HD_APP_NAME" >/dev/null 2>&1; then
    current=$(pacman -Q "$HD_APP_NAME" | awk '{print $2}')
    ok "$HD_APP_NAME $current is installed"
    if repo_reachable; then
      add_package_repo
      info "updates come with 'sudo pacman -Syu'"
    else
      info "the fork repository is not reachable; update by hand when it is back"
    fi
    return 0
  fi

  if repo_reachable; then
    add_package_repo
    info "installing from the fork's repository"
    if ! run pacman -S --needed --noconfirm "$HD_APP_NAME"; then
      warn "the repository install failed; falling back to the release package"
      install_release_package
      return 0
    fi
    note "installed $HD_APP_NAME from $REPO_URL"
  else
    warn "the fork repository does not answer at $REPO_URL"
    warn "installing the latest release package instead (no repo is added)"
    install_release_package
  fi
}

repo_reachable() {
  curl -fsSL --max-time 15 -o /dev/null "$REPO_URL/$HD_REPO_NAME.db" 2>/dev/null
}

add_package_repo() {
  [[ -r /etc/pacman.conf ]] || die "/etc/pacman.conf is missing: pacman is not set up"
  if grep -qE "^[[:space:]]*\[$HD_REPO_NAME\]" /etc/pacman.conf; then
    local configured=""
    configured=$(awk -v name="$HD_REPO_NAME" '
      $0 ~ "^[[:space:]]*\\[" name "\\]" { inside = 1; next }
      inside && $0 ~ "^[[:space:]]*\\[" { inside = 0; next }
      inside && $0 ~ "^[[:space:]]*Server[[:space:]]*=" {
        line = $0
        sub(/^[^=]*=[[:space:]]*/, "", line)
        print line
        exit
      }
    ' /etc/pacman.conf)
    if [[ -n $configured && $configured != "$REPO_URL" ]]; then
      warn "the [$HD_REPO_NAME] repository is already configured as $configured; keeping that one"
    else
      ok "the [$HD_REPO_NAME] repository is already in /etc/pacman.conf"
    fi
    return 0
  fi
  info "adding [$HD_REPO_NAME] to /etc/pacman.conf (unsigned; SigLevel Optional TrustAll)"
  local tmp
  tmp="$WORK_DIR/pacman.conf.$$"
  {
    cat /etc/pacman.conf
    printf '\n# Hearthdeck Hero: added by the bootstrap. Delete this block to stop using it.\n'
    printf '[%s]\n' "$HD_REPO_NAME"
    printf 'SigLevel = Optional TrustAll\n'
    printf 'Server = %s\n' "$REPO_URL"
  } >"$tmp"
  commit_file "/etc/pacman.conf" "$tmp" "root:root" "0644"
  [[ $HD_WROTE -eq 1 ]] && note "added the [$HD_REPO_NAME] repository to /etc/pacman.conf"
  run pacman -Sy --noconfirm
}

install_release_package() {
  local tag pkgver url file
  if [[ $DRY_RUN -eq 1 ]]; then
    say "  ${C_DIM}[dry-run]${C_RESET} would install the latest release package into /usr/bin"
    return 0
  fi
  tag=$(curl -fsSL -o /dev/null -w '%{url_effective}' "$HD_RELEASES/latest" 2>/dev/null) || tag=""
  tag="${tag##*/}"
  if [[ -z $tag || $tag != v* ]]; then
    die "could not find the latest release at $HD_RELEASES/latest"
  fi
  pkgver="${tag#v}"
  pkgver="${pkgver//-/.}"
  url="$HD_RELEASES/download/$tag/hearthdeck-hero-$pkgver-1-x86_64.pkg.tar.zst"
  file="$WORK_DIR/$(basename -- "$url")"
  info "downloading $url"
  if ! run curl -fsSL --max-time 600 -o "$file" "$url"; then
    die "could not download $url"
  fi
  if ! pacman -Qip "$file" >/dev/null 2>&1; then
    die "the downloaded file is not an Arch package: $file"
  fi
  run pacman -U --needed --noconfirm "$file"
  note "installed Hearthdeck Hero $tag from the release package"
}

step_mpv() {
  section "mpv"
  local user="$HD_APP_USER" home
  home=$(home_or_default "$user")
  install_mpv_config "$user" "$home/.config/mpv"
}

# install_mpv_config USER DIR — writes the gamepad bindings and the player
# settings as managed blocks, so lines a person adds to these files stay put.
# mpv reads both files from its config directory by default, which is where the
# app's player looks for them (`crates/watch-player` passes no --no-config).
install_mpv_config() {
  local user="$1" dir="$2" owner="$1:$1" wrote=0
  info "gamepad bindings and player settings: $dir"
  run install -d -o "$user" -g "$user" -m 0755 "$dir"

  write_mpv_block "$dir/input.conf" "$owner" <<'INPUT'
# --- Face Buttons (A, B, X, Y) ---
GAMEPAD_ACTION_DOWN    cycle pause              # A: Play / Pause
GAMEPAD_ACTION_RIGHT   cycle mute               # B: Mute / Unmute
GAMEPAD_ACTION_LEFT    cycle sub                # X: Cycle Subtitles
GAMEPAD_ACTION_UP      cycle fullscreen         # Y: Toggle Fullscreen

# --- D-Pad (Seeking & Volume) ---
GAMEPAD_DPAD_LEFT      seek -5                  # D-Pad Left: Rewind 5s
GAMEPAD_DPAD_RIGHT     seek 5                   # D-Pad Right: Fast-forward 5s
GAMEPAD_DPAD_UP        add volume 5             # D-Pad Up: Volume Up
GAMEPAD_DPAD_DOWN      add volume -5            # D-Pad Down: Volume Down

# --- Bumpers & Triggers (Chapters & Speed) ---
GAMEPAD_LEFT_SHOULDER  add chapter -1           # LB: Previous Chapter
GAMEPAD_RIGHT_SHOULDER add chapter 1            # RB: Next Chapter
GAMEPAD_LEFT_TRIGGER   multiply speed 0.9091    # LT: Slow Down Playback
GAMEPAD_RIGHT_TRIGGER  multiply speed 1.1       # RT: Speed Up Playback

# --- Analog Sticks (Fine Seek, Subs & Reset) ---
GAMEPAD_LEFT_STICK_LEFT   seek -30              # Left Stick Left: Rewind 30s
GAMEPAD_LEFT_STICK_RIGHT  seek 30               # Left Stick Right: Fast-forward 30s
GAMEPAD_LEFT_STICK_UP     no-osd sub-seek 1     # Left Stick Up: Next Subtitle Line
GAMEPAD_LEFT_STICK_DOWN   no-osd sub-seek -1    # Left Stick Down: Previous Subtitle Line
GAMEPAD_LEFT_STICK        set speed 1.0         # L3 (Click Left Stick): Reset Speed to 1.0x
GAMEPAD_RIGHT_STICK       show-progress         # R3 (Click Right Stick): Show Timeline / OSD

# --- Menu & View Buttons ---
GAMEPAD_BACK           cycle audio              # View (Back): Cycle Audio Tracks
GAMEPAD_START          quit-watch-later         # Menu (Start): Save Position & Quit
INPUT
  [[ $HD_WROTE -eq 1 ]] && wrote=1

  write_mpv_block "$dir/mpv.conf" "$owner" <<'MPVCONF'
# --- Input Settings ---
input-gamepad=yes

# --- Video & Audio ---
profile=high-quality
volume=80
volume-max=150

# --- Subtitles ---
sub-auto=fuzzy
slang=eng
MPVCONF
  [[ $HD_WROTE -eq 1 ]] && wrote=1

  if [[ $wrote -eq 1 ]]; then
    note "mpv: gamepad bindings and player settings for $user"
  else
    ok "mpv gamepad bindings already in place"
  fi
}

# write_mpv_block FILE OWNER — replaces the mpv block with stdin. The markers
# are added here: `ensure_block` expects them as part of the block.
write_mpv_block() {
  local file="$1" owner="$2" block
  block="$WORK_DIR/mpv.$$.$RANDOM"
  {
    printf '%s\n' "$HD_MPV_BEGIN"
    cat
    printf '%s\n' "$HD_MPV_END"
  } >"$block"
  if ! ensure_block "$file" "$HD_MPV_BEGIN" "$HD_MPV_END" "$owner" <"$block"; then
    rm -f -- "$block"
    die "could not write $file; fix or delete its hearthdeck-hero markers and re-run"
  fi
  rm -f -- "$block"
}

step_retroarch() {
  section "RetroArch"
  local user="$HD_APP_USER" home cfgdir cfg wrote=0
  home=$(home_or_default "$user")
  cfgdir="$home/.config/retroarch"
  cfg="$cfgdir/retroarch.cfg"

  info "controller profiles: $cfgdir/autoconfig/udev"
  info "settings: $cfg"
  run install -d -o "$user" -g "$user" -m 0755 "$cfgdir"
  install_autoconfig "$user" "$cfgdir" || true

  set_kv "$cfg" video_fullscreen '"true"' "$user:$user"
  [[ $HD_WROTE -eq 1 ]] && wrote=1
  set_kv "$cfg" video_windowed_fullscreen '"false"' "$user:$user"
  [[ $HD_WROTE -eq 1 ]] && wrote=1
  if [[ $wrote -eq 1 ]]; then
    note "retroarch.cfg: fullscreen on, windowed fullscreen off"
  else
    ok "retroarch.cfg already asks for fullscreen"
  fi
}

# install_autoconfig USER CFGDIR — RetroArch matches a controller profile by
# the device's name, so every Xbox and PlayStation family file is copied; a
# profile for a pad you do not own is never used.
install_autoconfig() {
  local user="$1" cfgdir="$2" dest="$2/autoconfig/udev"
  local system_dir="/usr/share/libretro/autoconfig/udev"
  local family="${CONTROLLER,,}"
  local src tarball udev file base
  local -a wanted=()

  if [[ $REFRESH_AUTOCONFIG -eq 0 && -d $system_dir ]] &&
    compgen -G "$system_dir/*.cfg" >/dev/null; then
    ok "RetroArch already ships controller profiles in $system_dir"
    return 0
  fi
  if [[ $REFRESH_AUTOCONFIG -eq 0 && -d $dest ]] && compgen -G "$dest/*.cfg" >/dev/null; then
    ok "controller profiles already installed in $dest"
    return 0
  fi

  if [[ $DRY_RUN -eq 1 ]]; then
    say "  ${C_DIM}[dry-run]${C_RESET} would download the $family profiles into $dest"
    return 0
  fi

  src="$WORK_DIR/autoconfig"
  tarball="$WORK_DIR/autoconfig.tar.gz"
  mkdir -p "$src"
  info "downloading the libretro controller profiles"
  if ! curl -fsSL --max-time 120 -o "$tarball" "$HD_AUTOCONFIG_TARBALL"; then
    warn "could not download the controller profiles; RetroArch will use its own defaults"
    return 1
  fi
  tar -xzf "$tarball" -C "$src" --wildcards '*/udev/*.cfg' 2>/dev/null || true
  udev=$(find "$src" -type d -name udev -print -quit)
  if [[ -z $udev ]]; then
    warn "the controller profile archive did not contain a udev folder"
    return 1
  fi

  for file in "$udev"/*.cfg; do
    [[ -f $file ]] || continue
    base=$(basename -- "$file")
    case "${base,,}" in
      *xbox* | *x-box*)
        [[ $family == "playstation" ]] || wanted+=("$file")
        ;;
      *playstation* | *dualshock* | *dualsense* | *ps3* | *ps4* | *ps5*)
        [[ $family == "xbox" ]] || wanted+=("$file")
        ;;
    esac
  done

  if [[ ${#wanted[@]} -eq 0 ]]; then
    warn "no $family controller profiles found in the archive"
    return 1
  fi
  run install -d -o "$user" -g "$user" -m 0755 "$dest"
  local copied=0
  for file in "${wanted[@]}"; do
    base=$(basename -- "$file")
    if [[ -f $dest/$base ]] && cmp -s -- "$file" "$dest/$base"; then
      continue
    fi
    install -m 0644 -o "$user" -g "$user" "$file" "$dest/$base"
    copied=$((copied + 1))
  done
  if [[ $copied -gt 0 ]]; then
    note "installed $copied controller profile(s) for $family pads"
  else
    ok "controller profiles were already up to date"
  fi
}

step_audio() {
  section "Sound"
  local user="$HD_APP_USER" home cfgdir unit wants card="$AUDIO_CARD" profile="$AUDIO_PROFILE"
  home=$(home_or_default "$user")
  cfgdir="$home/.config/systemd/user"
  unit="$cfgdir/hearthdeck-audio-profile.service"
  wants="$cfgdir/default.target.wants"

  if user_in_group "$user" audio; then
    ok "$user is in the audio group"
  else
    if getent group audio >/dev/null 2>&1; then
      run usermod --append --groups audio "$user"
      note "added $user to the audio group"
    else
      warn "the audio group does not exist"
    fi
  fi

  if [[ -z $card && -f $unit ]]; then
    card=$(sed -n 's|^ExecStart=.*set-card-profile \([^ ]*\) .*|\1|p' "$unit")
    profile=$(sed -n 's|^ExecStart=.*set-card-profile [^ ]* \(.*\)|\1|p' "$unit")
    if [[ -n $card ]]; then
      ok "a sound card profile is already pinned ($card, $profile)"
    fi
  fi

  if [[ -z $card ]]; then
    if ! user_audio_session "$user"; then
      warn "no PipeWire session for $user right now, so the card cannot be listed"
      warn "log in as $user (or reboot) and re-run, or pass --audio-card/--audio-profile"
      return 0
    fi
    choose_audio_profile "$user" || return 0
    card="$AUDIO_CARD"
    profile="$AUDIO_PROFILE"
  fi

  if [[ -z $profile ]]; then
    warn "no profile chosen for $card; nothing to pin"
    return 0
  fi

  info "pinning $card -> $profile at every login"
  write_if_changed "$unit" "$user:$user" "0644" <<EOF
[Unit]
Description=Hearthdeck Hero: sound card profile for the console
After=pipewire.service wireplumber.service

[Service]
Type=oneshot
ExecStart=/usr/bin/pactl set-card-profile $card $profile

[Install]
WantedBy=default.target
EOF
  [[ $HD_WROTE -eq 1 ]] && note "pinned the sound card profile $card -> $profile"

  run install -d -o "$user" -g "$user" -m 0755 "$wants"
  run ln -sfn ../hearthdeck-audio-profile.service "$wants/hearthdeck-audio-profile.service"

  if user_audio_session "$user"; then
    run pactl_as_user "$user" set-card-profile "$card" "$profile"
  else
    info "applied at the next login by the user unit"
  fi
}

# choose_audio_profile USER — lists the cards and lets the person pick one
# profile. Sets AUDIO_CARD and AUDIO_PROFILE.
choose_audio_profile() {
  local user="$1" card profile
  local -a cards=() profiles=()

  if [[ -n $AUDIO_CARD ]]; then
    if [[ -z $AUDIO_PROFILE ]]; then
      warn "--audio-card needs --audio-profile too"
      return 1
    fi
    return 0
  fi

  mapfile -t cards < <(pactl_as_user "$user" list cards 2>/dev/null | awk '/^\tName: / { print $2 }')
  if [[ ${#cards[@]} -eq 0 ]]; then
    warn "PipeWire reports no sound cards"
    return 1
  fi

  info "sound cards:"
  local i
  for i in "${!cards[@]}"; do
    printf '    %d) %s\n' "$((i + 1))" "${cards[i]}"
  done
  printf '    0) leave the profile alone\n'
  choose_number "Card" "${#cards[@]}" || return 1
  [[ $REPLY -eq 0 ]] && return 1
  card="${cards[REPLY - 1]}"

  mapfile -t profiles < <(pactl_as_user "$user" list cards 2>/dev/null | awk -v want="$card" '
    /^\tName: / { current = $2; inprofiles = 0; next }
    current == want && /^\tActive Profile: / { active = $3; next }
    current == want && /^\tProfiles:/ { inprofiles = 1; next }
    current == want && /^\t[A-Za-z]/ && !/^\tProfiles:/ { inprofiles = 0; next }
    current == want && inprofiles && /available: yes/ {
      line = $0
      sub(/^\t\t/, "", line)
      name = line
      sub(/:.*/, "", name)
      desc = line
      sub(/^[^:]*:[[:space:]]*/, "", desc)
      sub(/[[:space:]]*\(sinks:.*/, "", desc)
      printf "%s\t%s\n", name, desc
    }
    END { if (active != "") printf "ACTIVE\t%s\n", active }
  ')

  if [[ ${#profiles[@]} -eq 0 ]]; then
    warn "no available profile found for $card"
    return 1
  fi

  local active="" name desc entry
  local -a names=() descs=()
  for entry in "${profiles[@]}"; do
    if [[ $entry == "ACTIVE"$'\t'* ]]; then
      active="${entry#ACTIVE$'\t'}"
      continue
    fi
    IFS=$'\t' read -r name desc <<<"$entry"
    names+=("$name")
    descs+=("$desc")
  done
  info "profiles for $card${active:+ (active: $active)}:"
  for i in "${!names[@]}"; do
    printf '    %d) %s%s\n' "$((i + 1))" "${names[i]}" "${descs[i]:+  (${descs[i]})}"
  done
  printf '    0) leave the profile alone\n'
  choose_number "Profile" "${#names[@]}" || return 1
  [[ $REPLY -eq 0 ]] && return 1
  profile="${names[REPLY - 1]}"

  AUDIO_CARD="$card"
  AUDIO_PROFILE="$profile"
  note "chose the sound card profile $card -> $profile"
}

step_mounts() {
  section "External storage"
  local user="$HD_APP_USER" uid gid spec uuid path fstype label size dev
  uid=$(user_uid "$user")
  gid=$(user_gid "$user")

  if [[ ${#MOUNTS[@]} -eq 0 ]]; then
    local -a candidates=()
    local link
    for link in /dev/disk/by-uuid/*; do
      [[ -L $link ]] || continue
      dev=$(readlink -f -- "$link") || continue
      fstype=$(blkid -s TYPE -o value "$dev" 2>/dev/null || true)
      [[ -n $fstype ]] || continue
      # Anything already mounted (root, /boot, a swap partition) is not a
      # candidate for a new fstab entry.
      findmnt --source "$dev" >/dev/null 2>&1 && continue
      uuid="${link##*/}"
      label=$(blkid -s LABEL -o value "$dev" 2>/dev/null || true)
      size=$(lsblk -dno SIZE "$dev" 2>/dev/null || true)
      candidates+=("$uuid|$dev|$fstype|$label|$size")
    done

    if [[ ${#candidates[@]} -eq 0 ]]; then
      ok "no unmounted disks found; nothing to mount"
      return 0
    fi
    info "unmounted filesystems:"
    local i
    for i in "${!candidates[@]}"; do
      IFS='|' read -r uuid dev fstype label size <<<"${candidates[i]}"
      printf '    %d) %s  %s  %s  %s  (%s)\n' "$((i + 1))" "$dev" "$fstype" "$size" "${label:-no label}" "$uuid"
    done
    printf '    0) skip external storage\n'
    choose_number "Mount which filesystem" "${#candidates[@]}" || return 0
    [[ $REPLY -eq 0 ]] && return 0
    IFS='|' read -r uuid dev fstype label size <<<"${candidates[REPLY - 1]}"
    local default_path="/mnt/external01"
    prompt "Mount point" "$default_path" || return 0
    path="$REPLY"
    mount_add "$uuid" "$path" "$fstype" "$uid" "$gid"
    return 0
  fi

  for spec in "${MOUNTS[@]}"; do
    local parsed
    if ! parsed=$(parse_mount_spec "$spec"); then
      warn "--mount wants UUID=<uuid>=/absolute/path, got '$spec'"
      continue
    fi
    IFS=$'\t' read -r uuid path <<<"$parsed"
    dev=$(blkid -U "$uuid" 2>/dev/null || true)
    if [[ -z $dev ]]; then
      warn "no filesystem with UUID $uuid is plugged in; skipped"
      continue
    fi
    fstype=$(blkid -s TYPE -o value "$dev" 2>/dev/null || true)
    mount_add "$uuid" "$path" "$fstype" "$uid" "$gid"
  done
}

# mount_add UUID PATH FSTYPE UID GID
mount_add() {
  local uuid="$1" path="$2" fstype="$3" uid="$4" gid="$5"
  local unit line

  if fstab_has_uuid "$uuid"; then
    ok "UUID=$uuid is already in /etc/fstab"
    findmnt --target "$path" --noheadings --output SOURCE,TARGET,FSTYPE 2>/dev/null || true
    return 0
  fi
  if [[ -e $path ]] && findmnt --target "$path" >/dev/null 2>&1; then
    warn "$path is already a mount point; not touching it"
    return 0
  fi

  line=$(fstab_line "$uuid" "$path" "$fstype" "$uid" "$gid")

  info "adding to /etc/fstab:"
  info "$line"
  local tmp
  tmp="$WORK_DIR/fstab.$$"
  {
    cat /etc/fstab
    printf '\n# Hearthdeck Hero: added by the bootstrap.\n'
    printf '%s\n' "$line"
  } >"$tmp"
  commit_file "/etc/fstab" "$tmp" "root:root" "0644"
  if [[ $HD_WROTE -eq 0 ]]; then
    ok "/etc/fstab already had this entry"
    return 0
  fi
  note "added $path (UUID=$uuid) to /etc/fstab"
  run install -d -m 0755 "$path"

  if [[ $SYSTEMD -eq 1 ]]; then
    run systemctl daemon-reload
    unit=$(systemd-escape -p --suffix=automount "$path")
    if run systemctl restart "$unit"; then
      note "started $unit"
    else
      warn "$unit will be used at the next boot"
    fi
  else
    info "systemd is not running here: the mount is used at the next boot"
  fi
  run findmnt --target "$path" --noheadings --output SOURCE,TARGET,FSTYPE || true
}

# A profile that starts cage without the bootstrap's markers came from the
# hand-written setup (or someone's own attempt). It runs first and `exec`s, so
# the managed block below it would never run. It is not ours to delete, so say
# exactly what to do instead of guessing.
detect_legacy_kiosk() {
  local file="$1"
  [[ -f $file ]] || return 0
  grep -qxF -- "$HD_MARK_BEGIN" "$file" && return 0
  if grep -qE 'dbus-run-session[[:space:]]+cage|cage[[:space:]]+(-s|--)' "$file"; then
    warn "$file already starts cage, without the bootstrap's markers"
    warn "that block runs first and execs, so the new block below it will not run"
    warn "rename it aside (mv $file $file.old) and re-run --only session to switch"
    note "kept the unmanaged cage block already in $file"
  fi
}

step_session() {
  section "Console session"
  local user="$HD_APP_USER" home dm
  local -a managers=()
  home=$(home_or_default "$user")

  if ! command -v cage >/dev/null 2>&1; then
    warn "cage is not installed; the console session will not start"
  fi

  mapfile -t managers < <(enabled_display_managers)
  if [[ ${#managers[@]} -gt 0 ]]; then
    info "display managers that would take the screen: ${managers[*]}"
    if confirm "Disable them so the console can own the TV? (systemctl enable <name> brings one back)" y; then
      for dm in "${managers[@]}"; do
        run systemctl disable --now "$dm.service"
      done
      note "disabled display managers: ${managers[*]}"
    else
      warn "leaving ${managers[*]} enabled: the console may not start at boot"
    fi
  else
    ok "no display manager is enabled"
  fi

  if systemctl is-enabled --quiet seatd.service 2>/dev/null; then
    info "cage uses systemd-logind here, so seatd is not needed"
    run systemctl disable --now seatd.service
    note "disabled seatd"
  fi

  local block="$WORK_DIR/bash-profile.$$"
  detect_legacy_kiosk "$home/.bash_profile"
  cat >"$block" <<'PROFILE'
# >>> hearthdeck-hero kiosk >>>
# Managed by the Hearthdeck Hero bootstrap. Re-run it to update this block;
# delete the block to get a plain login shell back.
# On the first virtual terminal, hand the screen to cage and the console.
if [ -z "${WAYLAND_DISPLAY:-}" ] && [ "$(tty 2>/dev/null)" = "/dev/tty1" ] && [ -x /usr/bin/hearthdeck-hero ]; then
  export LIBSEAT_BACKEND=logind
  export BROWSER=luakit
  console_log="$HOME/.local/state/hearthdeck/kiosk.log"
  mkdir -p "$HOME/.local/state/hearthdeck"
  # The DRM node can appear a moment after getty starts; give it up to 10s.
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if [ -e /dev/dri/card0 ] || [ -e /dev/dri/card1 ]; then break; fi
    sleep 1
  done
  # A crash is retried, but not forever: after three tries the shell stays,
  # so the log can be read instead of watching the screen flicker.
  attempt=0
  while [ "$attempt" -lt 3 ]; do
    dbus-run-session cage -s -- /usr/bin/hearthdeck-hero --console >>"$console_log" 2>&1
    status=$?
    [ "$status" -eq 0 ] && break
    attempt=$((attempt + 1))
    printf '%s console exited with %s; retry %s/3\n' "$(date '+%F %T')" "$status" "$attempt" >>"$console_log"
    sleep 5
  done
  printf 'Hearthdeck Hero stopped. Log: %s\n' "$console_log" >&2
fi
# <<< hearthdeck-hero kiosk <<<
PROFILE
  # shellcheck disable=SC2002 # the helper wants stdin; a redirect reads better here
  if ! ensure_block "$home/.bash_profile" "$HD_MARK_BEGIN" "$HD_MARK_END" "$user:$user" <"$block"; then
    rm -f -- "$block"
    die "could not write the kiosk block in $home/.bash_profile; tty1 autologin was not enabled"
  fi
  [[ $HD_WROTE -eq 1 ]] && note "added the kiosk block to $home/.bash_profile"
  rm -f -- "$block"

  # Only after the block is in place: otherwise tty1 would autologin into a
  # plain shell and the TV would look broken.
  info "tty1 will log in $user and start cage + $HD_APP_BIN --console"
  write_if_changed "/etc/systemd/system/getty@tty1.service.d/autologin.conf" "root:root" "0644" <<EOF
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin $user --noclear %I \$TERM
EOF
  [[ $HD_WROTE -eq 1 ]] && note "enabled autologin for $user on tty1"
  if [[ $SYSTEMD -eq 1 ]]; then
    run systemctl daemon-reload
    run systemctl enable getty@tty1.service
  fi

  if [[ ! -e $home/.hushlogin ]]; then
    run install -o "$user" -g "$user" -m 0644 /dev/null "$home/.hushlogin"
    note "quiet login (no motd) for $user"
  fi

  if [[ ! -x $HD_APP_BIN ]]; then
    warn "$HD_APP_BIN is missing: the console will not start until the app is installed"
  fi
}

step_tmdb() {
  section "TMDB"
  local user="$HD_APP_USER" home file key
  home=$(home_or_default "$user")
  file="$home/.config/$HD_DATA_DIR/tmdb-api-key"
  if [[ -s $file ]]; then
    info "the TMDB key is already set ($file)"
    return 0
  fi
  say "  TMDB fills the film pages, its rails and the account's lists. Its API needs"
  say "  this app's own key, free from TMDB: themoviedb.org → your account →"
  say "  Settings → API → Request an API key → Developer. It is 32 characters,"
  say "  and it is stored on this machine only (0600), never in the shell history."
  if ! prompt "TMDB API key (empty to skip)" ""; then
    warn "no terminal to ask for the key: TMDB stays off (Settings → Watch → Providers says so)"
    return 0
  fi
  key=$(printf '%s' "$REPLY" | tr -d '[:space:]')
  if [[ -z $key ]]; then
    note "no key given: TMDB stays off; re-run with --only tmdb to set it later"
    return 0
  fi
  if [[ ! $key =~ ^[0-9a-fA-F]{32}$ ]]; then
    warn "that is not 32 hexadecimal characters; saving it anyway"
  fi
  if [[ $DRY_RUN -eq 1 ]]; then
    say "  [dry-run] would write $file"
    return 0
  fi
  # Written with printf, not `run`: the key must never reach the log.
  install -d -m 700 -o "$user" -g "$(id -gn "$user")" "$(dirname "$file")"
  if printf '%s\n' "$key" >"$file"; then
    chown "$user" "$file" && chmod 600 "$file"
    info "the TMDB key is set for $user"
  else
    warn "could not write $file"
  fi
}

step_lockdown() {
  section "Lockdown"
  local user="$HD_APP_USER"
  local -a others=()

  if ! user_in_group "$user" wheel; then
    ok "$user is not an administrator"
    return 0
  fi

  mapfile -t others < <(other_wheel_members "$user")
  if [[ ${#others[@]} -eq 0 ]] || ! wheel_can_sudo; then
    warn "not removing $user from wheel: no other account could administer this machine"
    warn "create one first, for example: sudo useradd -m -G wheel operator && sudo passwd operator"
    warn "then re-run the bootstrap with --lockdown"
    note "kept administrator rights for $user (no other admin was found)"
    return 0
  fi

  info "other administrators: ${others[*]}"
  if [[ $LOCKDOWN -eq 0 ]]; then
    if [[ $ASSUME_YES -eq 1 ]]; then
      note "kept administrator rights for $user (pass --lockdown to remove them)"
      return 0
    fi
    if ! confirm "Remove $user from wheel now? Afterwards it cannot use sudo." n; then
      note "kept administrator rights for $user (declined)"
      return 0
    fi
  fi
  run gpasswd -d "$user" wheel
  note "removed $user from wheel; ${others[*]} keeps sudo"
}

# ---------------------------------------------------------------------------
# Preflight, plan, summary
# ---------------------------------------------------------------------------

preflight() {
  local os_id="" os_like=""
  say ""
  info "checking the machine"

  [[ ${EUID} -eq 0 ]] || die "run it as root, for example: curl -fsSL <url> | sudo bash -s --"
  [[ -r /etc/os-release ]] || die "no /etc/os-release: this does not look like Linux"
  # shellcheck source=/dev/null
  . /etc/os-release
  os_id="${ID:-}"
  os_like="${ID_LIKE:-}"
  if [[ $os_id != "arch" && $os_like != *arch* ]]; then
    die "this bootstrap supports Arch-based systems only (found '${os_id:-unknown}')"
  fi
  [[ $(uname -m) == "x86_64" ]] || die "only x86_64 packages are published (found $(uname -m))"
  command -v pacman >/dev/null 2>&1 || die "pacman is missing"

  if command -v systemctl >/dev/null 2>&1 && systemctl show-environment >/dev/null 2>&1; then
    SYSTEMD=1
  else
    SYSTEMD=0
    warn "systemd is not running: units are written but not started"
  fi

  if [[ ! -r /dev/tty ]]; then
    warn "no terminal: questions cannot be asked (pass -y to accept the defaults)"
  fi

  local free
  free=$(df --output=avail /var 2>/dev/null | tail -n 1 | tr -d ' ') || free=""
  if [[ -n $free ]] && ((free < 5 * 1024 * 1024)); then
    warn "less than 5 GB free on /var; the packages need a few GB"
  fi

  if [[ ! -d /dev/dri ]]; then
    warn "no /dev/dri: the app and RetroArch need a GPU to draw"
  fi

  if curl -fsS --max-time 10 -o /dev/null https://github.com 2>/dev/null; then
    ok "network reachable"
  else
    warn "github.com is not reachable: package and profile downloads may fail"
  fi
}

build_step_list() {
  local step candidate keep
  SELECTED=()
  for step in user packages aur ludusavi app tmdb mpv retroarch audio mounts session lockdown; do
    keep=1
    if [[ ${#ONLY[@]} -gt 0 ]]; then
      local matched=0
      for candidate in "${ONLY[@]}"; do
        if [[ $candidate == "$step" ]]; then matched=1; fi
      done
      if [[ $matched -eq 0 ]]; then keep=0; fi
    fi
    for candidate in "${SKIP[@]}"; do
      if [[ $candidate == "$step" ]]; then keep=0; fi
    done
    if [[ $keep -eq 1 ]]; then
      SELECTED+=("$step")
    fi
  done
  STEP_TOTAL=${#SELECTED[@]}
}

plan() {
  say ""
  say "${C_BOLD}Plan${C_RESET}"
  say "  account     $HD_APP_USER (created if missing, added to video, audio, input, render)"
  say "  packages    session, RetroArch, the libretro cores, mpv, rclone, luakit, Dolphin"
  say "  AUR         ${HD_PKGS_AUR[*]}"
  say "  Ludusavi    per-game emulator saves for the emulators that are installed"
  say "  app         $HD_APP_NAME from $REPO_URL (or the latest release package)"
  say "  mpv         gamepad bindings and player settings for $HD_APP_USER"
  say "  RetroArch   $CONTROLLER controller profiles, fullscreen on"
  say "  sound       the card profile you pick, re-applied at every login"
  say "  storage     ${#MOUNTS[@]} disk(s) given on the command line, or picked interactively"
  say "  session     tty1 autologin -> cage -> $HD_APP_BIN --console"
  if [[ $LOCKDOWN -eq 1 ]]; then
    say "  tmdb        this app's TMDB API key, asked once (free from themoviedb.org), kept 0600"
    say "  lockdown    $HD_APP_USER loses sudo at the end, if another admin exists"
  else
    say "  lockdown    asks before taking sudo away; --yes never does"
  fi
  say ""
  say "  steps: ${SELECTED[*]}"
  say "  a re-run only changes what is missing or different."
}

confirm_plan() {
  if [[ $ASSUME_YES -eq 1 ]]; then
    return 0
  fi
  if ! confirm "Proceed?" y; then
    die "nothing was changed"
  fi
}

summary() {
  say ""
  say "${C_BOLD}Summary${C_RESET}"
  if [[ ${#CHANGES[@]} -gt 0 ]]; then
    if [[ $DRY_RUN -eq 1 ]]; then
      say "  a real run would:"
    fi
    local change
    local -A seen=()
    for change in "${CHANGES[@]}"; do
      [[ -n ${seen[$change]:-} ]] && continue
      seen[$change]=1
      say "  • $change"
    done
  else
    say "  nothing needed changing: the machine already matched the plan"
  fi

  say ""
  say "${C_BOLD}What to do next${C_RESET}"
  say "  • reboot when you are ready: sudo systemctl reboot"
  say "  • the TV starts the console by itself; on a keyboard, Ctrl+Alt+F2 gives a terminal"
  say "  • sign in to Epic, GOG and Amazon Games in Settings → Stores"
  say "  • add your ROM folders in Settings → Emulators, then scan"
  say "  • point Ludusavi at a backup folder (Settings → Saves) once you have saves"
  say "  • hook up rclone for sync: rclone config  (use --auth-no-open-browser over SSH)"
  if user_in_group "$HD_APP_USER" wheel; then
    say "  • $HD_APP_USER still has sudo: run the bootstrap again with --lockdown to remove it"
  else
    say "  • updates need an administrator: sign in as one and run sudo pacman -Syu"
  fi

  say ""
  local check
  for check in "$HD_APP_BIN" /usr/bin/cage /usr/bin/retroarch /usr/bin/mpv; do
    if [[ -x $check ]]; then
      say "  ${C_GREEN}✓${C_RESET} $check"
    else
      say "  ${C_YELLOW}!${C_RESET} $check is missing"
    fi
  done
  if [[ $LOGGING -eq 1 ]]; then
    say ""
    say "  log: $LOG_FILE"
  fi
  say ""
}

# ---------------------------------------------------------------------------
# Arguments
# ---------------------------------------------------------------------------

usage() {
  cat <<'USAGE'
Hearthdeck Hero bootstrap: sets up an Arch-based machine as the TV console.

  curl -fsSL https://raw.githubusercontent.com/viniciuscr/hearthdeck-hero/fork/main/scripts/bootstrap.sh | sudo bash -s --

It creates the console account, installs the packages and Hearthdeck Hero
itself, configures RetroArch, the sound card, external disks and the tty1
console session, and can take sudo away from the console account at the end.

It asks before anything risky, backs up every file it edits, and is safe to
run again: a re-run only changes what is missing or different.

Options:
  -y, --yes                 answer yes to every question (except lockdown)
      --dry-run             print the plan and each change; change nothing
      --user NAME           console account (default hearthdeck-hero)
      --controller SET      xbox, playstation or both (default both)
      --refresh-autoconfig  re-download RetroArch controller profiles
      --audio-card NAME     sound card for the console (with --audio-profile)
      --audio-profile NAME  card profile to apply at every login
      --mount UUID=<uuid>=/path   mount a filesystem (repeatable)
      --no-mount            do not look for external disks
      --app-package FILE    install this .pkg.tar.zst instead of the repo
      --repo URL            package repository (default: the fork's)
      --lockdown            remove the console account from wheel at the end
      --no-update           leave pacman -Syu for later
      --skip STEP           skip a step (repeatable)
      --only STEP           run only these steps (repeatable)
      --log FILE            transcript path (default /var/log/hearthdeck-bootstrap.log)
  -h, --help                this text
  -V, --version             print the bootstrap version

Steps, in order:
  user packages aur ludusavi app mpv retroarch audio mounts session lockdown

Examples:
  sudo bash bootstrap.sh --dry-run
  sudo bash bootstrap.sh --controller playstation --mount UUID=86B6-D516=/mnt/external01
  sudo bash bootstrap.sh --lockdown -y
USAGE
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -y | --yes) ASSUME_YES=1 ;;
      --dry-run) DRY_RUN=1 ;;
      --user)
        shift
        HD_APP_USER="${1:-}"
        [[ -n $HD_APP_USER ]] || die "--user needs a name"
        [[ $HD_APP_USER =~ ^[a-z_][a-z0-9_-]{0,31}$ ]] ||
          die "--user takes a plain account name, not '$HD_APP_USER'"
        ;;
      --controller)
        shift
        CONTROLLER="${1:-}"
        case "$CONTROLLER" in
          xbox | playstation | both) ;;
          *) die "--controller takes xbox, playstation or both" ;;
        esac
        ;;
      --refresh-autoconfig) REFRESH_AUTOCONFIG=1 ;;
      --audio-card)
        shift
        AUDIO_CARD="${1:-}"
        [[ -n $AUDIO_CARD ]] || die "--audio-card needs a card name"
        [[ $AUDIO_CARD != *[[:space:]]* ]] || die "--audio-card cannot contain whitespace"
        ;;
      --audio-profile)
        shift
        AUDIO_PROFILE="${1:-}"
        [[ -n $AUDIO_PROFILE ]] || die "--audio-profile needs a profile name"
        [[ $AUDIO_PROFILE != *[[:space:]]* ]] || die "--audio-profile cannot contain whitespace"
        ;;
      --mount)
        shift
        MOUNTS+=("${1:-}")
        [[ -n ${1:-} ]] || die "--mount needs UUID=/path"
        ;;
      --no-mount) SKIP+=("mounts") ;;
      --app-package)
        shift
        APP_PACKAGE="${1:-}"
        [[ -n $APP_PACKAGE ]] || die "--app-package needs a file"
        ;;
      --repo)
        shift
        REPO_URL="${1%/}"
        [[ -n $REPO_URL ]] || die "--repo needs a URL"
        [[ $REPO_URL =~ ^https?://[^[:space:]]+$ ]] || die "--repo takes an http(s) URL"
        ;;
      --lockdown) LOCKDOWN=1 ;;
      --no-update) NO_UPDATE=1 ;;
      --skip)
        shift
        local -a skip_steps=()
        IFS=',' read -r -a skip_steps <<<"${1:-}"
        SKIP+=("${skip_steps[@]}")
        ;;
      --only)
        shift
        local -a only_steps=()
        IFS=',' read -r -a only_steps <<<"${1:-}"
        ONLY+=("${only_steps[@]}")
        ;;
      --log)
        shift
        LOG_FILE="${1:-}"
        LOGGING=1
        ;;
      -h | --help)
        usage
        exit 0
        ;;
      -V | --version)
        printf '%s %s\n' "$HD_APP_NAME" "$HD_BOOTSTRAP_VERSION"
        exit 0
        ;;
      *)
        die "unknown option '$1' (try --help)"
        ;;
    esac
    shift
  done
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

start_log() {
  if [[ $DRY_RUN -eq 1 ]]; then
    LOGGING=0
    return 0
  fi
  [[ -n $LOG_FILE ]] || LOG_FILE="/var/log/hearthdeck-bootstrap.log"
  if ! mkdir -p -- "$(dirname -- "$LOG_FILE")" 2>/dev/null; then
    warn "cannot create $(dirname -- "$LOG_FILE"): running without a transcript"
    LOGGING=0
    return 0
  fi
  if ! printf '\n===== %s — %s bootstrap %s =====\n' \
    "$(date '+%F %T')" "$HD_APP_NAME" "$HD_BOOTSTRAP_VERSION" >>"$LOG_FILE" 2>/dev/null; then
    warn "cannot write $LOG_FILE: running without a transcript"
    LOGGING=0
    return 0
  fi
  chmod 0600 -- "$LOG_FILE" 2>/dev/null || true
  LOGGING=1
}

run_step() {
  case "$1" in
    user) step_user ;;
    packages) step_packages ;;
    aur) step_aur ;;
    ludusavi) step_ludusavi ;;
    app) step_app ;;
    mpv) step_mpv ;;
    retroarch) step_retroarch ;;
    audio) step_audio ;;
    mounts) step_mounts ;;
    session) step_session ;;
    lockdown) step_lockdown ;;
    *) die "unknown step '$1'" ;;
  esac
}

main() {
  parse_args "$@"
  WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/hearthdeck-bootstrap.XXXXXX")
  RUN_STAMP=$(date +%Y%m%d-%H%M%S)
  trap cleanup EXIT
  trap 'exit 130' INT TERM
  trap 'exit 129' HUP
  start_log
  banner
  preflight
  build_step_list
  if [[ ${#SELECTED[@]} -eq 0 ]]; then
    die "no steps selected (check --skip/--only)"
  fi
  plan
  confirm_plan

  STEP_INDEX=0
  local step
  for step in "${SELECTED[@]}"; do
    run_step "$step"
  done
  summary
}

if [[ ${HD_BOOTSTRAP_LIB:-0} != "1" ]]; then
  main "$@"
fi
