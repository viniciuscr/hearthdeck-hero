# Bootstrapping the TV box

`scripts/bootstrap.sh` turns a fresh Arch-based machine (Arch, CachyOS,
EndeavourOS, Manjaro…) into the console the project is made for: tty1 boots
into `cage` and `hearthdeck-hero --console`, with RetroArch and its cores,
Stremio, mpv (with `yt-dlp`, which plays a YouTube trailer in Watch's Trailer
tab), Dolphin, PCSX2, rclone, luakit and Ludusavi installed and
configured.

It is one self-contained bash file, so it can be pasted as a single command
over SSH:

```bash
curl -fsSL https://raw.githubusercontent.com/viniciuscr/hearthdeck-hero/fork/main/scripts/bootstrap.sh | sudo bash -s --
```

The script is read from the pipe, so every question is asked on `/dev/tty`:
the SSH terminal stays usable while it runs. `sudo` is required; the script
refuses to start otherwise.

## Before the first run

- Arch-based, `x86_64`, with `pacman`, systemd and a network connection.
- A GPU with a Vulkan driver. The script installs the driver package for AMD
  or Intel it finds in `/sys/class/drm`; for NVIDIA it installs the Vulkan
  loader and tells you to add `nvidia`/`nvidia-open` for your kernel first.
- About 5 GB free on `/var` for the packages and the app.
- Another account that can `sudo` if you intend to let the script take the
  console account's admin rights away at the end (see [Lockdown](#lockdown)).

Start with a dry run. It prints the plan and every file it would write, asks
nothing, and changes nothing:

```bash
curl -fsSL https://raw.githubusercontent.com/viniciuscr/hearthdeck-hero/fork/main/scripts/bootstrap.sh | sudo bash -s -- --dry-run
```

Then run it for real, or from a clone: `sudo scripts/bootstrap.sh`.
## What it changes

The run is a sequence of steps, in this order. `--only` and `--skip` take these
names, comma separated (`--only app,retroarch`, `--skip audio,mounts`).

| Step | What it does | Files and units |
|---|---|---|
| `user` | Creates `hearthdeck-hero` (or `--user NAME`) with bash and the `video`, `audio`, `input`, `render` groups; offers to set a login password | `/etc/passwd`, `/etc/group`, `/home/<user>` |
| `packages` | Offers `pacman -Syu`, then installs the session, emulation, media and backup packages, plus `base-devel` and the GPU's Vulkan driver | pacman database |
| `aur` | Installs `stremio-service-bin`, `pcsx2-latest-bin` and `ludusavi` with `shelly`, `paru` or `yay`, falling back to `makepkg` as the console account | AUR builds under `~/.cache/hearthdeck-bootstrap` |
| `ludusavi` | Downloads the per-emulator manifests for the emulators that are installed and adds them, with each emulator's save folder, to Ludusavi's config | `~/.config/ludusavi/config.yaml`, `~/.config/ludusavi/manifests/` |
| `app` | Adds the fork's `[hearthdeck]` repository when it answers, then installs `hearthdeck-hero`; otherwise installs the latest release `.pkg.tar.zst` | `/etc/pacman.conf`, `/usr/bin/hearthdeck-hero` |
| `mpv` | Installs the gamepad bindings and player settings the player needs (A/B/X/Y, D-pad, sticks, bumpers, triggers) as a managed block | `~/.config/mpv/input.conf`, `~/.config/mpv/mpv.conf` |
| `retroarch` | Downloads the libretro joypad profiles for Xbox and/or PlayStation pads into the account's config, and sets fullscreen in `retroarch.cfg` | `~/.config/retroarch/autoconfig/udev`, `~/.config/retroarch/retroarch.cfg` |
| `audio` | Adds the account to `audio` and pins a sound card profile (asked once, or `--audio-card`/`--audio-profile`) so it is re-applied at every login | `~/.config/systemd/user/hearthdeck-audio-profile.service` |
| `mounts` | Offers the unmounted filesystems it finds (or takes `--mount UUID=<uuid>=/path`) and adds them to fstab with `nofail` and `x-systemd.automount` | `/etc/fstab`, `/mnt/…` |
| `tmdb` | Asks for this app's TMDB API key (free from themoviedb.org → Settings → API → Developer) and the optional v4 read token, and stores both 0600; each is skipped when its file already has one | `~/.config/hearthdeck/tmdb-api-key`, `~/.config/hearthdeck/tmdb-access-token` |
| `simkl` | Asks for this app's Simkl client id (free from simkl.com → Apps → Developers; an AUTH V2 *TV, devices & command line* app) and stores it 0600; skipped when the file already has one | `~/.config/hearthdeck/simkl-client-id` |
| `session` | Disables display managers (asks first), turns `seatd` off, enables tty1 autologin and adds the kiosk block to the account's profile | `/etc/systemd/system/getty@tty1.service.d/autologin.conf`, `~/.bash_profile`, `~/.hushlogin` |
| `lockdown` | Takes the console account out of `wheel`, but only when another account can still administer the machine | `/etc/group` |

A few deliberate choices:

- **`seatd` is not installed.** `cage` uses `systemd-logind` on these systems;
  an old `seatd` that is enabled is turned off instead.
- **Stremio needs no service of its own.** The app finds and starts the headless
  Stremio runtime when Watch needs it (`crates/watch/src/service.rs`), so the
  package is all the setup it needs.
- **The kiosk block restarts the console at most three times.** After a crash
  the log is on screen and in `~/.local/state/hearthdeck/kiosk.log`; after three
  failures the login shell stays, so the machine is debuggable instead of
  flickering.
- **libretro profiles are matched by device name**, so installing both the Xbox
  and PlayStation families is harmless: a profile for a pad you do not own is
  never used.

## Emulator saves, per game

Ludusavi's own manifest knows PC games; emulator saves are not in it. The
`ludusavi` step adds the per-game manifests from
[sharkusmanch/ludusavi-emulator-manifests](https://github.com/sharkusmanch/ludusavi-emulator-manifests)
for whichever emulators it finds installed, and points Ludusavi at the folder
each one writes its saves to:

| Emulator | Manifest | Root it needs |
|---|---|---|
| RetroArch (every core) | `retroarch.yaml` | `~/.config/retroarch` — the folder holding `saves/` and `states/` |
| Dolphin (GameCube, Wii) | `dolphin.yaml` | `~/.local/share/dolphin-emu` |
| PCSX2 (PS2) | `pcsx2.yaml` | `~/.config/PCSX2/memcards` (folder memory cards) |

The manifests are downloaded to `~/.config/ludusavi/manifests/` and referenced
by path, so backups work offline; a re-run refreshes them. Games appear with a
platform suffix — `Turbo Goose Racer (RetroArch)`, `Robot Gardener 2 (PS2)` —
so they never merge with a PC game of the same name.

The step edits `~/.config/ludusavi/config.yaml` in place: it adds `roots:`
entries of type `other` and `manifest.secondary` paths, and keeps everything
else (backup folder, format, retention). Ludusavi validates the result itself;
if it refuses, the previous file is put back. Ludusavi rewrites its own config
when it loads it, so the file may look reformatted afterwards — the version
from before the step is kept as `config.yaml.bak.<timestamp>`.

Only the emulators this bootstrap installs are detected. For another one (say
DuckStation, at `~/.local/share/duckstation`), add its manifest URL or path to
`manifest.secondary` and its save folder to `roots` — the repository lists the
file and folder for each emulator.

## Safe to run again

Every step reads the state first and changes only what is missing or differs:

- packages are checked with `pacman -Q` and installed with `--needed`;
- the managed blocks in `/etc/fstab`, `~/.bash_profile` and `retroarch.cfg` are
  replaced in place, never duplicated;
- a file is rewritten only when its content would change, and the previous
  copy is kept next to it as `<file>.bak.<YYYYmmdd-HHMMSS>`;
- `autologin.conf` and the audio unit are compared byte for byte;
- the AUR step is skipped when the packages are already installed.

A second run therefore reports "nothing needed changing" and writes nothing.
Run `--dry-run` on a machine you are unsure about: it reports the same "would
write" lines without touching anything.

## On a machine that is already set up

Yes — it is meant to be run on a half-configured box, because every step looks
at the machine first and the rest is a question. Three things are worth knowing:

- **Nothing is reinstalled just because it is missing from the plan.** An
  installed `hearthdeck-hero` is reported, not replaced (its own channel updates
  it); packages already present are skipped; controller profiles already in the
  account's config are left alone; a mount already in fstab is not added twice.
- **A full run also asks to upgrade the system** (`pacman -Syu`) and to disable
  enabled display managers. On a box that is working, answer no to either; the
  rest still runs. `--no-update` skips the upgrade without asking.
- **An older, hand-written kiosk block is reported, not touched.** If
  `~/.bash_profile` already starts cage without the bootstrap's markers, it runs
  before the managed block and `exec`s, so the managed one is never reached. The
  run warns and prints the `mv` that gets you onto the managed block.

If the app was installed by hand (a binary in `~/.local/bin` rather than a
package), a run that includes the `app` step installs the system package into
`/usr/bin` — the path the console session starts. `--skip app` leaves the
existing install alone.

The smallest useful run on a partially configured machine is the missing piece,
by name:

```bash
sudo bash bootstrap.sh --dry-run                       # what would change
sudo bash bootstrap.sh --only mpv                      # just the mpv config
sudo bash bootstrap.sh --only mpv --user some-account  # if the console account differs
```

## Questions

Anything risky is a question, with the safe answer as the default:

- update the system before installing (`pacman -Syu`);
- set a password for the console account;
- disable the display managers;
- change the sound card profile;
- mount an external disk;
- remove the console account from `wheel`.

With no terminal, the script cannot ask and will **not** guess: it skips the
question and carries on. For an unattended run, pass `-y`, which accepts the
defaults (the lockdown step still needs `--lockdown`).

## Options

```
-y, --yes                 answer yes to every question (except lockdown)
    --dry-run             print the plan and each change; change nothing
    --user NAME           console account (default hearthdeck-hero)
    --controller SET      xbox, playstation or both (default both)
    --refresh-autoconfig  re-download RetroArch controller profiles
    --audio-card NAME     sound card for the console (with --audio-profile)
    --audio-profile NAME  card profile to apply at every login
    --mount UUID=<uuid>=/path   mount a filesystem (repeatable)    --no-mount            do not look for external disks
    --app-package FILE    install this .pkg.tar.zst instead of the repo
    --repo URL            package repository (default: the fork's)
    --lockdown            remove the console account from wheel at the end
    --no-update           leave pacman -Syu for later
    --skip STEP           skip a step (repeatable, comma separated)
    --only STEP           run only these steps (repeatable, comma separated)
    --log FILE            transcript (default /var/log/hearthdeck-bootstrap.log)
-h, --help                usage
-V, --version             bootstrap version
```

Examples:

```bash
# the dry run, on a machine you do not own yet
sudo bash bootstrap.sh --dry-run

# a PlayStation pad and one external SSD
sudo bash bootstrap.sh --controller playstation --mount UUID=86B6-D516=/mnt/external01

# unattended, and no admin rights for the console account at the end
sudo bash bootstrap.sh -y --lockdown
```

## Lockdown

The console account never needs permanent `sudo`: the AUR step borrows it
through `/etc/sudoers.d/99-hearthdeck-bootstrap`, which is removed on the way
out — normal exit, failure, Ctrl+C, or a dropped SSH session. If a run is
killed outright and leaves the file behind, the next run stops and tells you to
remove it rather than silently keeping a passwordless rule.

`--lockdown` removes the account from `wheel` at the end, so the TV user can
run the console and nothing else. Without the flag the script asks; with `-y`
it never takes the rights away on its own.

It refuses, and keeps the rights, unless another account is in `wheel` **and**
`%wheel` can actually `sudo` in `/etc/sudoers` or `/etc/sudoers.d`. Create that
account first:

```bash
sudo useradd -m -G wheel operator && sudo passwd operator
sudo bash bootstrap.sh --lockdown
```

After lockdown, **updates need that administrator**: `sudo pacman -Syu` over
SSH, or from the other account. The app's own updater uses `pkexec` to install
a downloaded package, which also asks for an administrator — that is the price
of the console account not being one.

## What is still manual

The bootstrap installs and configures; it cannot log in for you.

- **Stores:** Epic Games, GOG and Amazon Games are signed in from Settings →
  Stores (the Epic window is a separate binary, already installed).
- **Emulators:** add your ROM folders in Settings → Emulators, then scan; the
  cores and RetroArch are already there.
- **rclone:** `rclone config` (or `rclone config --auth-no-open-browser` over
  SSH, and `BROWSER=luakit rclone config` on the TV) sets up the cloud the
  Sync page uses.
- **Ludusavi:** point it at a backup folder in Settings → Saves once you have
  saves to back up. Emulator saves are already per game thanks to the manifests
  above; a PC game and an emulated game of the same name stay separate.
- **Controller:** RetroArch autodetects it from the profiles, and the gamepad
  bindings for Watch playback are installed for you (the blocks under
  `~/.config/mpv`). Check both once on the TV.

## Undo

Each part is independent; remove only what you want to undo.

| Change | Undo |
|---|---|
| Display managers disabled | `sudo systemctl enable --now sddm` (or `gdm`, `lightdm`, `cosmic-greeter`…) |
| tty1 autologin | `sudo rm /etc/systemd/system/getty@tty1.service.d/autologin.conf && sudo systemctl daemon-reload` |
| Kiosk block | Delete the `# >>> hearthdeck-hero kiosk >>>` … `# <<< hearthdeck-hero kiosk <<<` block from `~/.bash_profile` |
| Sound card profile | `rm ~/.config/systemd/user/hearthdeck-audio-profile.service ~/.config/systemd/user/default.target.wants/hearthdeck-audio-profile.service` |
| External mount | Remove the `UUID=… /mnt/…` line and its `# Hearthdeck Hero:` comment from `/etc/fstab`, then `sudo systemctl daemon-reload` |
| Ludusavi emulator manifests | Remove the `secondary` entries and the emulator roots from `~/.config/ludusavi/config.yaml` (or restore its `.bak` file); `~/.config/ludusavi/manifests/` can be deleted |
| Fork repository | Delete the `[hearthdeck]` block and its comment from `/etc/pacman.conf` |
| RetroArch settings | `~/.config/retroarch/retroarch.cfg.bak.<timestamp>` holds the previous file; the controller profiles are in `~/.config/retroarch/autoconfig/udev` |
| mpv gamepad bindings | Delete the `# >>> hearthdeck-hero gamepad >>>` … `# <<< hearthdeck-hero gamepad <<<` block from `~/.config/mpv/input.conf` and `~/.config/mpv/mpv.conf` |
| Packages | `sudo pacman -Rns hearthdeck-hero retroarch …` (the AUR ones too) |
| Admin rights back | `sudo usermod -aG wheel hearthdeck-hero` from an administrator |

Every file the script edited has a `.bak.<timestamp>` copy next to it; the
whole transcript is in `/var/log/hearthdeck-bootstrap.log`.

## Troubleshooting

**The console does not start.** On a keyboard, `Ctrl+Alt+F2` gives a second
terminal (autologin is only on tty1) and `Ctrl+Alt+F1` goes back. The app's own
output is in `~/.local/state/hearthdeck/kiosk.log`; the session units are in
`journalctl -b -u getty@tty1`. The usual causes are a missing Vulkan driver, a
display manager that is still enabled, or `cage` failing on a machine without a
usable DRM node (`/dev/dri`).

**RetroArch is windowed or the picture is wrong.** The bootstrap sets
`video_fullscreen = "true"` and `video_windowed_fullscreen = "false"`. If the
screen still misbehaves, change the video driver inside RetroArch
(Settings → Driver), or edit `~/.config/retroarch/retroarch.cfg`; re-running the
bootstrap with `--only retroarch` only touches those two keys.

**The controller works in the console but not while watching.** Watch playback
belongs to mpv, which needs `input-gamepad=yes` and the `GAMEPAD_*` key names;
both live in the `~/.config/mpv` block (`--only mpv` restores it). If the app's
settings say the controller is unsupported, that mpv build has no SDL gamepad
support (`mpv --input-gamepad=yes --version` fails): the config cannot add it.

**An emulated game's saves do not show up.** The root must be the folder the
manifest expects: RetroArch's config folder (the one with `saves/` and
`states/`), PCSX2's `memcards`, Dolphin's user folder. If the emulator was
pointed somewhere else, change that root in `~/.config/ludusavi/config.yaml`.
`ludusavi backup --preview --api </dev/null` lists what it would back up.

**There is no sound.** Check `wpctl status` as the console account. The pinned
profile is re-applied by a user unit at every login, so a change made by hand
may be overwritten. To pick another one, name it and re-run the step:
`sudo bash bootstrap.sh --only audio --audio-card <card> --audio-profile <profile>`
(the step reuses an existing choice instead of asking again; delete
`~/.config/systemd/user/hearthdeck-audio-profile.service` to be asked again).

**An AUR package failed to build.** The output is kept under
`~/.cache/hearthdeck-bootstrap` until the script finishes (then the folder is
removed; the details are in the log). Install it by hand, then re-run with
`--only aur`.

**The external disk is not mounted.** Buttons: `findmnt /mnt/external01`,
`systemctl status mnt-external01.automount`, `journalctl -b | grep -i mount`.
Because the entry has `nofail` and `x-systemd.automount`, an unplugged disk
never blocks the boot; it mounts when it appears and the path is touched.

**The fork repository is unreachable.** The bootstrap then installs the latest
release package and does *not* add `[hearthdeck]` to `pacman.conf` (a dead
server would make every `pacman -Syu` noisy). Re-run `--only app` once
<https://viniciuscr.github.io/hearthdeck-hero/arch/x86_64/hearthdeck.db>
answers. See [docs/RELEASING.md](RELEASING.md) for the one-time Pages setup.

**pacman says "failed to prepare transaction (could not satisfy
dependencies)".** The package database is newer than the installed system (the
usual result of `--no-update`, or of a machine that has not been upgraded in a
while). Run `sudo pacman -Syu`, then re-run the bootstrap.

## Security notes

- The fork's Arch repository is unsigned (`repo-add` without `--sign`), so the
  `pacman.conf` block uses `SigLevel = Optional TrustAll`. That is the existing
  channel described in [docs/RELEASING.md](RELEASING.md); the project should
  sign the repository before it is used outside a private TV machine.
- The AUR step creates `/etc/sudoers.d/99-hearthdeck-bootstrap` for the console
  account, validated with `visudo -cf`, and removes it on normal exit, on
  failure, and on `INT`/`TERM`/`HUP` (`trap`). It is only created when an AUR
  package is missing, and a leftover file from a killed run makes the next run
  stop with the exact `rm` command.
- The script downloads three things: the fork's package (or its repository
  database), the libretro joypad profiles, and AUR sources through your helper.
  Everything else comes from `pacman`.
- Nothing in the script is compiled or hidden: it is plain bash, readable end to
  end before you pipe it into `sudo`.

## Changing the script

It lives at `scripts/bootstrap.sh` on `fork/main`; the URL in this document
points there, so publishing is a merge to `fork/main` (see
[docs/FORK.md](FORK.md)). Keep the helpers (`write_if_changed`, `ensure_block`,
`set_kv`, `commit_file`) as the only way to touch a file: they are what makes a
re-run harmless, and `HD_BOOTSTRAP_LIB=1 source scripts/bootstrap.sh` exposes
them to a test harness.

## TMDB key

TMDB authenticates every call with an *application* key: the app carries one, and
a person using it only ever approves a session (by scanning a QR code from
Settings → Watch → Accounts). The key therefore belongs to the machine, and this
step is where it is asked for:

- **Where to get it**: log in to themoviedb.org → your account → **Settings →
  API** → *Request an API key* → **Developer**. It is approved immediately and is
  32 characters.
- **Where it goes**: `~/.config/hearthdeck/tmdb-api-key`, mode 0600, owned by the
  console account. Nothing else is touched, and the key is never logged.
- **Setting it later**: `scripts/bootstrap.sh --only tmdb`. The app adopts the file
  into its credential store the first time Settings opens, so no rebuild is
  needed; without it, Settings → Watch → Accounts says the key is missing and
  refuses to pretend a Connect would work.
- **The v4 read token is a second file**, `~/.config/hearthdeck/tmdb-access-token`
  (the long value on the same TMDB page). It is optional and only powers the
  account's "recommended for you" rails, so an empty answer skips it. The key and
  the token are asked for independently: a re-run that finds the key already
  there still asks for a missing token, and `--dry-run` reports both without
  writing either.
- **Skipping it**: answer empty. TMDB stays off; everything else works.

## Simkl client id

Simkl is a tracker: it keeps what you watch in step across devices. Like TMDB it
authenticates every call with an *application* credential — a **client id** —
and the person's part is a code approved from the TV:

- **Where to get it**: log in to simkl.com → your account → **Apps →
  Developers** and create an app of type **TV, devices & command line** (the
  device/PIN flow needs an AUTH V2 app; see <https://api.simkl.org>). The
  `client_id` it shows is public and safe to keep on the machine; create it once
  per installation (or once for the fork, if the release carries it).
- **Where it goes**: `~/.config/hearthdeck/simkl-client-id`, mode 0600, owned by
  the console account. Nothing else is touched, and the id is never logged.
- **Setting it later**: `scripts/bootstrap.sh --only simkl`. The app adopts the
  file into its credential store the first time Settings opens; without it,
  Settings → Watch → Accounts says the client id is missing and refuses to
  pretend a Connect would work.
- **How the person signs in**: Settings → Watch → Accounts → **Connect Simkl**
  is the only button, and it opens a pairing panel: the eight-character code at
  the top, `https://simkl.com/pin?user_code=…` as a QR under it, and the wait
  under that. The phone approves it and the panel closes on its own — the app
  polls every interval Simkl returns (five seconds), slows down when asked to,
  rides out a network fault, and gives up when the code's fifteen minutes are
  gone. The tokens are kept in `~/.config/hearthdeck/simkl-auth.json` (0600).
- **Skipping it**: answer empty. Simkl stays off; everything else works.
