# Windows plug-ins on Linux: troubleshooting

NI's plug-ins (Massive, Raum, Replika, Phasis…) and many others have no Linux
versions. On Linux, DECKARD runs their Windows versions through
[Wine](https://www.winehq.org) and [yabridge](https://github.com/robbert-vdh/yabridge):
you install the plug-in into Wine, yabridge makes a Linux VST3 bundle for it in
`~/.vst3/yabridge`, and DECKARD hosts that bundle like any other plug-in.
**Preferences ▸ Plug-ins ▸ Windows plug-ins** shows each piece and installs what's
missing. This page is for when a piece doesn't do its part.

How the pieces fit:

| Piece                        | Where                                                     |
| ---------------------------- | --------------------------------------------------------- |
| Wine                         | `wine --version`                                          |
| yabridge                     | `~/.local/share/yabridge` (`yabridgectl`, `libyabridge-vst3.so`, `yabridge-host.exe`) |
| Plug-ins installed into Wine | `~/.wine/drive_c/Program Files/Common Files/VST3`         |
| yabridge's bundles for them  | `~/.vst3/yabridge/<name>.vst3`, made by `yabridgectl sync` |
| DECKARD's plug-in list       | Preferences ▸ Plug-ins, after **Rescan**                   |
| yabridge's own log           | `/tmp/yabridge.log` when started as shown below            |

## A plug-in I installed doesn't show up after Rescan

Check each step in this order; the first one that fails is the problem.

1. **Is it in Wine?**

   ```
   ls ~/.wine/drive_c/Program\ Files/Common\ Files/VST3
   ```

   If its `.vst3` isn't there, it isn't installed. NI's standalone installers are
   zips holding a `… Setup PC.exe`; run that with Wine:

   ```
   cd ~/Downloads && unzip -o Raum_Installer.zip && wine "Raum 1.3.7 Setup PC.exe"
   ```

   (Native Access, run the same way, installs products too.) A `.vst3` here may be a
   folder or a single file; both are fine.

2. **Has yabridge made its bundle?**

   ```
   ls ~/.vst3/yabridge
   ```

   If the plug-in is in Wine but not here, run a sync. DECKARD does it before each
   rescan when **Sync before each rescan** is on (it is by default), or press **Sync
   now**; by hand:

   ```
   ~/.local/share/yabridge/yabridgectl sync
   ```

   It says how many plug-ins it set up. If it says the folder isn't managed, add it
   first: `yabridgectl add "$HOME/.wine/drive_c/Program Files/Common Files/VST3"`.

3. **Does DECKARD search that folder?** Preferences ▸ Plug-ins lists the folders it
   searches; the standard ones (`~/.vst3`, the system's `vst3` folders, `~/.vst`) are
   always there and marked "Standard". `~/.vst3/yabridge` is inside `~/.vst3`, so it
   is covered.

4. **Rescan.** Then look at the list; a plug-in switched off there has its checkbox
   cleared.

5. **Did the scan fail on it?** The status line after a rescan says `couldn't scan
   <name>: …` when a bundle didn't load in the scanner. Run the scan by hand to see
   the plug-in's own output:

   ```
   deckard --scan-vst3 ~/.vst3/yabridge/Raum.vst3
   ```

   A plug-in that loads prints its classes; one that doesn't prints Wine's and
   yabridge's errors. A missing Windows library (`err:module:…`) means a Wine
   component the plug-in needs isn't there; `winetricks` installs most.

## The plug-in's window opens but takes no clicks or typing

Symptoms: the window draws and its licence or settings dialog appears, but nothing
happens when you click its buttons, and you can't type into its fields. Knobs don't
move either.

**Cause: Wine 9.22 or later with yabridge's 5.1.1 release.** yabridge's last release
(November 2024) predates a change in how Wine 9.22 handles windows; through it, mouse
and keyboard events never reach the plug-in (yabridge issues
[#382](https://github.com/robbert-vdh/yabridge/issues/382) and
[#409](https://github.com/robbert-vdh/yabridge/issues/409)). The fix is in yabridge's
`master` branch and not yet in a release, so yabridge has to be built from its
source.

Check: `wine --version` says 9.22 or later (`wine-11.0`), and
`~/.local/share/yabridge/BUILD` doesn't exist (DECKARD writes it when it builds
yabridge).

**Fix, in DECKARD:** Preferences ▸ Plug-ins ▸ Windows plug-ins says "newer than
yabridge's 5.1.1 release" next to the Wine version. Press **Install build tools…**
(asks for an administrator's password; installs the compiler, Meson, Ninja, cargo and
Wine's development files), then **Build yabridge**. The build takes a few minutes;
when it's done DECKARD syncs the bundles again. Open the plug-in's window again.

**Fix, by hand:** the same script DECKARD runs, `build-yabridge.sh` (in `scripts/` in
the repository, `/usr/share/deckard` from the packages, or next to `deckard` in the
tarball):

```
./build-yabridge.sh --check     # lists what it needs, as your distribution's packages
sudo apt install <those>        # or dnf / pacman
./build-yabridge.sh
~/.local/share/yabridge/yabridgectl sync
```

On Debian, Ubuntu and Mint with Wine from WineHQ's packages, the development files
are `wine-stable-dev` (or `wine-staging-dev`, `wine-devel-dev`, matching the variant
installed); with the distribution's Wine, `libwine-dev`. On Fedora, `wine-devel`. The
release copy is kept as `~/.local/share/yabridge.release`; move it back over
`~/.local/share/yabridge` to undo.

The other way out is Wine 9.21 or earlier, which yabridge's README explains how to
install.

## Seeing what yabridge sees

yabridge logs everything the host and the plug-in say to each other, and with
`+editor` its window handling too. Start DECKARD like this, open the plug-in's
window, do what fails, and read the log:

```
YABRIDGE_DEBUG_FILE=/tmp/yabridge.log YABRIDGE_DEBUG_LEVEL=2+editor deckard
```

What to look for:

- `Initializing yabridge version …` and `wine version: …` at the top: which yabridge
  and Wine are really in use.
- `IPlugView::attached(parent = …)`, then `Reparenting … succeeded`: the plug-in's
  window went into DECKARD's.
- `FocusIn for window … (wine window active)` and `Not grabbing input focus … (already
  focused)` when the mouse enters the window: keyboard focus is where it should be.
  If these say `inactive` or focus goes to another window, the desktop isn't giving
  DECKARD's plug-in window the focus.
- `IEditController::beginEdit` / `performEdit` when you turn a knob: clicks reach the
  plug-in. None of them in a session where you clicked on controls means they don't
  (the Wine 9.22 problem above).
- `err:` lines from Wine: a missing component. `fixme:` lines are normal.

Holding **Shift** while moving the mouse into the plug-in's window makes yabridge
focus the Wine window directly, which some plug-ins' dialogs need for typing (the
yabridge README's "Input focus grabbing").

## Nothing works at all: check the basics

- `wine --version` runs. If not, Preferences installs Wine, or your package manager
  does (`sudo apt install wine`, `sudo dnf install wine`, `sudo pacman -S wine`).
- `~/.local/share/yabridge/yabridgectl status` lists the managed folders and each
  plug-in's state.
- A plug-in that loads in another host (REAPER, Bitwig) through the same yabridge but
  not in DECKARD is DECKARD's problem: open an issue with the log above.
