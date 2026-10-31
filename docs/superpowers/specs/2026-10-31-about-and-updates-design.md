# About page, update notice and self-update

Status: approved 2026-10-31, built on the `about-and-updates` branch (on top of
`splice-link`).

## Why

DECKARD's About box is a plain window with the version and the licences. It should
tell who made it, who helped, how to support it, and whether a newer release exists;
and it should install that release, on Linux, macOS and Windows, so nobody has to find
the releases page.

## The About window

A frameless window, 560 × 316, centred, closed with its × or Escape, opened from
Help ▸ About DECKARD (and the macOS application menu) as today.

**Background.** `apps/viewer/assets/about.gif` (the Blade Runner Japan GIF, 220 × 129,
135 KB) fills the window, scaled up with linear filtering, looping at the GIF's own
frame delays. Frames are decoded once, the first time the window opens, into one
texture per frame.

**Glass card.** Inset 24 px from the window's edges, corner radius 14. It is made of:

1. the current GIF frame, blurred (two passes of a box blur, radius 6 at GIF scale,
   done once per decoded frame), drawn clipped to the card's rounded rectangle;
2. a white tint at 10 % over that;
3. a 1 px edge: white at 55 % along the top and left, black at 35 % along the bottom
   and right, so the card reads as a lit slab of glass.

All text sits on the card, white, with a 1 px dark shadow so it stays readable on any
frame.

**Content, top to bottom.**

- The DECKARD wordmark (the existing `deckard_logo`) and the version (`VERSION`).
- "Created by Josh Summitt" → https://github.com/summitt.
- "More from Panic-Signals" → https://github.com/Panic-Signals.
- Contributors: avatars (28 px, round) in a row, login on hover, each a link to the
  profile. Fetched from GitHub when the window opens; "contributors: offline" when that
  fails. Fetched once per launch.
- Update line: "You have the latest" / "Checking…" / "Update available: 0.2.0", with
  the **Update** button (see below) and a link to the release page; "Updates are off"
  when the preference is off; nothing for a build that isn't from a release.
- Sponsor row, three buttons: **Splice** (`splice::url(None)`), **Patreon**
  (https://www.patreon.com/c/null0perat0r), **GitHub Sponsors**
  (https://github.com/sponsors/panic-signals).
- A collapsed "Licences and trademarks" section with today's notes (NI, Plaits, Link,
  the MI licence text).

## The update notice

Once per launch, when `Settings::check_updates` (default on; Preferences ▸ General:
"Check for updates at launch") is on and `VERSION` parses as a release version, a
thread fetches `https://api.github.com/repos/Panic-Signals/Deckard/releases/latest`
and compares its `tag_name` with `VERSION` as numbers (`major.minor.patch`, a leading
`v` dropped; anything else is "not a release version", never nagged). A newer release
sets `App::update: Option<Release>`.

When set: a 6 px dot, Maschine's accent colour, at the top right of the header's
DECKARD wordmark, with hover text "Update available: 0.2.0 (Help ▸ About)"; and the
About card's update line and button.

## Self-update

Pressing **Update** downloads the asset for this install, verifies it, installs it and
relaunches. The old install stays until the new one is in place; any failure leaves it
untouched and shows the reason on the card.

**Which asset.** From the release's assets, by file-name pattern (not by version
text):

| Install                                     | Asset                              |
| ------------------------------------------- | ---------------------------------- |
| Linux, exe under `/usr` and dpkg present    | `deckard_*_amd64.deb` (`arm64`)    |
| Linux, exe under `/usr` and rpm present     | `deckard-*.x86_64.rpm` (`aarch64`) |
| Linux, elsewhere (the tarball)              | `deckard-*-linux-<arch>.tar.gz`    |
| macOS (`…/DECKARD.app/Contents/MacOS/…`)    | `DECKARD-*-macos-universal.zip`    |
| Windows                                     | `DECKARD-*-windows-x64.zip`        |

No matching asset: "no build for this system in 0.2.0".

**Download.** To a temporary folder beside the install (same file system, so the swap
is a rename), with progress ("12 of 48 MB") on the card. Verified against the asset's
`digest` (`sha256:…`) from the API; a mismatch is a failure.

**Install.**

- Linux package: `pkexec sh -c 'apt-get install -y ./deckard_….deb'` (or
  `dnf install -y`), as the Wine installer does; exit 126 is "cancelled".
- Linux tarball: `tar xzf` into the temp folder; the unpacked `deckard-<v>-linux-<arch>`
  folder is renamed over the install folder (the old one is first renamed to
  `<name>.old`, deleted after the new one starts).
- macOS: `ditto -x -k` (keeps the signature) into the temp folder; the new
  `DECKARD.app` is swapped with the running one the same way; relaunch with `open -n`.
- Windows: `tar -xf` (Windows 10's bsdtar reads zip) into the temp folder; a
  `update.cmd` is written that waits for this process to exit, moves the new files over
  the old, starts the new exe and deletes itself; the app starts it and quits.

**Relaunch.** Linux and macOS start the new exe (same arguments: the open project is
saved first, as on quit) and exit. Windows: `update.cmd` does it.

**Dev builds** (`VERSION` is the crate's, not a release's) never check or update.

## Release pipeline fixes

- The version is the release's **tag** (then `inputs.version`), not its title: the
  0.1.0 release shipped as `0.1.0-Early-Alpha-Release` because the title was used.
- The Windows build fails under MSVC on stmlib's `__attribute__`: `crates/mi/build.rs`
  force-includes `src/msvc.h`, which defines `__attribute__(x)` away, when the
  compiler is MSVC. Verified by a `workflow_dispatch` run of the release workflow.

## Code

- `apps/viewer/src/github.rs`: `Release { version: (u32,u32,u32), tag, url, assets:
  Vec<Asset { name, url, digest, size }> }`, `latest_release()`, `contributors()`,
  `fetch(url) -> Vec<u8>` (ureq, 20 s), `parse_version(&str)`, JSON parsing with
  serde_json. Pure parsing functions are tested with fixture JSON.
- `apps/viewer/src/about.rs`: the window; `Gif` (decode with the `image` crate's
  `gif` feature; frames, delays, blurred copies), `blur()` tested on a tiny image, the
  card painting, contributors/avatars state (`Receiver`s).
- `apps/viewer/src/update.rs`: `Install::detect()`, `pick_asset(&Release, &Install)`
  (tested with the 0.1.0 asset names), `run(release, install, progress) -> Result<(),
  String>` on a thread, the per-OS install steps, `relaunch()`.
- `App`: `update: Option<Release>`, `update_check: Option<Receiver<…>>`,
  `about: Option<about::State>`; `Settings::check_updates`.
- `Cargo.toml` (viewer): `ureq = { version = "3", features = ["rustls"] }` (check the
  default features; no native TLS), `sha2`, `image` gains `"gif"`.
- README: "Sponsoring DECKARD" section with the three links; the About and update
  behaviour in "Getting DECKARD".

## Not in this

Delta updates, update channels (beta), rollback after a bad launch, automatic install
without pressing Update, Linux Arch/Flatpak/AppImage packaging.
