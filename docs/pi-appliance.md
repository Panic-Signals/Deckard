# Raspberry Pi appliance (work in progress)

> **Status:** planned, not started. Feature 4 in `FEATURES.md`. The VST2 host it builds on
> is in (`crates/plugins/src/vst2.rs`); nothing below is built yet.

A disk image for a Raspberry Pi, made by a build step. Flash it to an SD card, put it in
the Pi with the MK3 and a USB audio interface, and DECKARD starts by itself, like an MPC:
no login, no desktop, nothing else running.

## Hardware

- **A Raspberry Pi 5 with 4 GB of memory at least; 8 GB recommended.** 1 GB is too
  little. Large sample sets and several plug-in instances want 8 GB. Stem separation is
  the heaviest feature: off on 4 GB boards, to be tested on 8 GB before it's promised.
  The image is the same 64-bit system for every size, and should boot on a Pi 4 too.
- **Any class-compliant USB audio interface** (MOTU's current ones are). Linux's own
  USB audio driver covers them, and DECKARD already talks to ALSA directly. The Pi 5
  has no headphone jack; HDMI audio is only a fallback.
- **A monitor is optional:** a small portable one can be plugged in at any time.

## The image

- **Built from Raspberry Pi OS Lite** (Debian, 64-bit) with Raspberry Pi's own image
  builder (`rpi-image-gen`, or `pi-gen` as a fallback), not Buildroot: much less to
  keep up for nearly the same result.
- **A script, `scripts/build-pi-image.sh`,** installs the arm64 `.deb` that
  `scripts/build-linux.sh` already makes (with the MK3's udev rule) and a short list of
  packages, and writes `deckard-pi-<version>.img.xz` to flash with Raspberry Pi
  Imager. It can run in CI (GitHub's ARM runners) or in Docker.
- **Partitions:**
  - the system, read-only, so pulling the power can't corrupt it;
  - a `DECKARD` data area that grows to fill the card on first boot, holding
    `Projects/`, `VST2 Plugins/`, `VST3/`, `Samples/` and `Updates/`;
  - the small boot area, which any computer can read and write.

## Starting DECKARD

- **A systemd service** runs DECKARD as its own user at boot and restarts it if it
  quits. No login prompt, no desktop.
- **DECKARD runs full screen in `cage`,** a display server that runs one program.
- **HDMI is forced on at boot** at a fixed resolution, so DECKARD has a screen to draw
  on whether or not a monitor is connected. It runs the same either way, and a monitor
  plugged in later just shows it. No separate headless mode is needed. To check on
  hardware: the resolution a monitor plugged in later gets.
- **Screen size:** the window is laid out for about 1500 points across. A 1080p monitor
  is fine; a 1024 × 600 one would need the interface scaled.
- **Touch monitors** work through `cage` with DECKARD's tap and drag (full multi-touch is
  Feature 14).
- **Audio:** DECKARD uses the first USB interface it finds unless Preferences names one,
  with a small default buffer. CPU at full speed and real-time priority for the audio
  thread; a real-time kernel (as the MPC has) can come later, since it means building
  our own kernel.

## Getting files on and off

- **A network share** of the data area (Samba), advertised so it shows up as
  `deckard.local` in Finder and Explorer: drop in projects, samples and plug-ins over
  Wi-Fi or Ethernet. Anyone on the same network can reach a share without a password,
  so a password can be set in the boot area's `deckard.txt`, and the docs say so plainly.
- **Wi-Fi without logging in:** Ethernet works at once. Wi-Fi's name and password go in
  `deckard.txt` on the boot area, editable from any computer before first boot (and
  Raspberry Pi Imager's own Wi-Fi settings, if they can be supported). Later: choosing
  Wi-Fi from the MK3.
- **USB drives mount by themselves.** DECKARD looks in `<drive>/VST2 Plugins` (the MPC's
  layout, which the VST2 host already reads), `<drive>/VST3` and `<drive>/Projects`,
  scans again when a drive comes or goes, and shows the drive in the browser. This needs
  a change in DECKARD.
- **Updates:** a new `.deb` dropped into `Updates/` on the share is installed at the next
  boot. Reflashing always works too.
- **Shut Down** from the MK3, so the data area is never cut off mid-write.

## Order of work

1. An image that boots straight into DECKARD full screen, with the MK3 and a USB
   interface working and HDMI forced on. Checks the display and audio latency on
   hardware.
2. The data area and the network share.
3. USB drives (the DECKARD change) and Wi-Fi from `deckard.txt`.
4. Updates from `Updates/`, Shut Down, and the read-only system.

Each step is flashed and tried on a Pi; the build can be type-checked here, but an image
can only be tested on the hardware.

## Plug-ins on the Pi

Only plug-ins built for Linux on 64-bit ARM run. Audio Units are Apple's and can't.

- **MPC plug-ins:** a Linux `.so` following the VST 2.4 interface, in a folder with a
  `plugin.json` naming a binary per CPU, as inMusic's public MPC plug-in SDK
  (MIT-licensed) describes them. DECKARD's VST2 host reads those folders and loads the
  `arm64` build; the SDK's builds need only glibc 2.31.
- **VST3 plug-ins** with an `aarch64-linux` build load as they do on any Linux.
- Commercial plug-ins sold for the MPC may check for MPC hardware; that's for each
  vendor to say.
