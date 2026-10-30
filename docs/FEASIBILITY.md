# Maschine 3 → Rust: reverse-engineering feasibility

Target: `/Applications/Native Instruments/Maschine 3/Maschine 3.app` (universal, arm64 slice
sha256 `cadb263f…0a58ba`). All numbers below come from the arm64 slice and the tools in
`tools/re-tools`; re-run them to reproduce.

## What's in the binary

| | |
|---|---|
| Main executable | 578 MB universal; arm64 slice 286 MB |
| Machine code (`__text`) | 76 MB, ~289k functions |
| Symbols | **~500k, not stripped** (full C++ names and signatures) |
| UI framework | Qt 6.8.5 (namespaced `NI_6_8_5_R2`), statically linked, plus QML |
| Scripting | Lua via sol2; hardware controller logic is written in Lua |
| Third-party | Qt, libc++, boost, OpenSSL, SQLite, FreeType/HarfBuzz, Xerces, TagLib, zmq, zplane élastique, Ableton Link, VST3 SDK, iZotope stem-separation bundle |

Approximate code-size split (`out/inventory/code-size-by-namespace.tsv`):

- ~70% third-party or generated: Qt 25%, sol2/Lua binding glue 21%, std/boost 10%, compiled QML 3%, shader tooling 4%
- **NI-authored code is only ~8–12 MB** of arm64:
  - `NI::MASCHINE::GUI` 2.1 MB, `NI::MASCHINE::DATA` 1.9 MB (document model)
  - `NI::MASCOMMON::*` 1.5 MB, `NI::MASCHINE::HW` 0.3 MB, `NI::MASCHINE::RT` + `MASCOMMON::RT` 0.5 MB (realtime engine)
  - `NI::DSP` (88 classes: drum synths, EQ/comp/reverbs, Jam FX), `NI::DB3` (library browser), `NI::ML` (Lua host)

## Recovered as *source*, not machine code

| Artifact | Tool | Result |
|---|---|---|
| Hardware controller logic (MK3, Studio, Mikro MK3, Jam, MH1071, KK S2, …) | `ni-res-extract` | **445 Lua files, ~102k lines** |
| NWL widget stylesheets / layouts for hardware screens | `ni-res-extract` | 184 files, ~41k lines |
| Desktop UI | `qrc-extract` | **NI QML ~22k lines** (Maschine, MASCommon, Lumen, NativeControlSuite, ModulationEditor) |
| Pictures, fonts, strings (en/de/jp), DSP tables (IFFT cymbal, modal noise), metronome | both | ~2,700 PNG/WebP/SVG, 50 fonts, 10 WAV |

Lua calls **~1,640 distinct engine APIs** (`NI.DATA.*` document model, `NI.HW.*` hardware,
`NI.GUI.*`). That list is effectively the spec for the data-model API a port must provide.

## Formats and protocols

- **Projects (`.mxprj`)**: NI `hsin`/`DSIN` chunk container wrapping a `PROJMAS2` payload that is a
  `boost::serialization` archive of `NI::MASCHINE::DATA::*` objects (353 registered types in
  `out/inventory/serializable-types.txt`). There are 119 `.mxprj` files on this machine to use as
  a differential corpus.
- **Hardware**: the app doesn't talk USB directly. It uses CFMessagePort IPC to
  `NIHardwareAgent` / `NIHostIntegrationAgent`. A port can either speak that IPC or drive the
  devices over USB HID directly (community work exists for MK3 / Mikro MK3).

## How hard is each piece

| Subsystem | Difficulty | Why |
|---|---|---|
| Hardware page logic | **Low–medium** | Decision: host NI's Lua unchanged in `mlua`, loaded from the user's install; the work is implementing the ~1,640 `NI.*` functions it calls |
| UI layout and look | **Low–medium** | QML + stylesheets are readable; reimplement in egui/slint |
| Project file reader | **Done (reading)** | Every local document decodes; field meanings still partly unnamed; writing not started |
| Data model (`NI.DATA`) | **Medium–high** | ~5k functions, but names and signatures survive; API surface already enumerated |
| Hardware I/O | **Medium** | Agent IPC or HID; observable with USB capture |
| Realtime engine (scheduling, voices, routing) | **High** | Behavior must be matched by listening/measurement, not just reading |
| NI DSP (drum synths, FX) | **High** | Hand-tuned SIMD; sound-matching is iterative |
| Licensed third-party (élastique stretch, iZotope stems, Komplete content, NKS) | **Replace** | Use open alternatives (e.g. Rubber Band / signalsmith-stretch, demucs) |

## Legal and method boundary

SoundCraft, the cited precedent, is **clean-room**: its `AGENTS.md` forbids reading or disassembling
the Pro Tools bundle and limits work to black-box observation. Treating decompiled NI code or the
extracted Lua/QML as source to translate would make this a derivative of NI's copyrighted code,
and NI's EULA restricts reverse engineering.

The defensible version:

1. Use extracted artifacts and symbols to **write specs** (feature lists, API surface, file-format
   field tables, hardware page behavior) in our own words.
2. Implement in Rust **from the specs**, ideally by a different agent/person than the one reading
   NI artifacts.
3. Don't ship NI assets (images, fonts, Lua, QML, sample content).
4. File-format and hardware interoperability is the strongest ground for direct RE.

## Tooling gaps found

| Gap | Status |
|---|---|
| No extractor for Qt `rcc` resources compiled into Mach-O | **Built**: `qrc-extract` (136 sets / 3,856 files in ~3 s) |
| No extractor for NI `compiled_resources` | **Built**: `ni-res-extract` (1,769 files, 0 unresolved) |
| No fast, whole-binary C++ class/API inventory | **Built**: `sym-inventory` (289k functions in ~4 s) |
| REA + Hopper: `open_binary` times out at 120 s on the 286 MB slice | Configurable: `REA_HOPPER_STARTUP_TIMEOUT_MS` (Ghidra: `REA_GHIDRA_STARTUP_TIMEOUT_MS`, default 330 s) |
| REA's Ghidra provider rejected under the default JDK 17 | Set `JAVA_HOME` to JDK 21+ (mise `temurin-21`) and `GHIDRA_HEADLESS_MAXMEM` (default 2 GB is too small) |
| `--snapshot` doesn't persist Ghidra analysis; every CLI call re-analyzes | REA always runs Ghidra with `-deleteProject`, so every new session re-runs the full 3 h analysis. Fix: keep our own persistent Ghidra project (`analyzeHeadless` without `-deleteProject`) and query it with `-process -noanalysis` |
| REA reports `cleanup_incomplete` after a successful long Ghidra run (`sysctl errno 22` reading process ownership) | REA bug; the result is still in `details.partial_observation`, and the processes did exit |
| REA + saved Hopper database (`HOPPER_TARGET_KIND=database`) never connects | Hopper limitation: the launcher's `-Y`/`-y` scripts never run when opening a `.hop` (tested with and without `-a`), and REA's bridge is a `-Y` script. The `.hop` itself is fine: it opens in the GUI in ~65 s using ~11–13 GB RAM. Use it interactively; script against `-e` loads only |
| boost::serialization / NI `hsin` container parser | **Built**: `crates/mxprj` decodes all 3,683 local projects/groups/sounds/presets completely; see `docs/mxprj-format.md` |
| Field order of 300+ serialized types | **Built**: `serialize-schema` (506 outlines in ~7 s) |
| Cross-references without a full analysis | **Built**: `callers` |
| Unnamed decoded fields | **Built**: `getter-offsets` (544 trivial accessors → class/offset/name) + `tools/name_fields.py` renamed 139 offset-named fields (e.g. Song+0x100 → `tempo`) |
| What each UI panel shows | **Built**: `gui-bindings` maps 261 GUI classes to the document-model functions they call (`out/gui/bindings.md`) |
| Decompiling without Ghidra's 3 h analysis | **Built**: `tools/ghidra/decompile.sh` imports with `-noanalysis` into a persistent project (2 min once), defines callee functions from raw `BL`s, then decompiles a list of symbols (506 functions in ~75 s) |
| sol2 binding table → Lua API spec (arg types per `NI.*` function) | Next: recover from the sol2 registration code |

## Ghidra vs. Hopper (free) — measured

Same target (NIHardwareAgent arm64 slice, 11 MB, 12.6k symbols), same function
(`MaschineHIDController::processPadEvents`), both via REA 6.4.0 CLI:

| | Ghidra 12.1.4 (JDK 21) | Hopper (free) |
|---|---|---|
| Open + analyze + decompile | 211 s, 1.7 GB RSS | 127 s |
| Re-run with snapshot | 197 s (no reuse) | n/a (free version can't save) |
| Pseudocode | 260 lines; real `switch`, typed locals, `/ 4095.0` | 755 lines; nested `if`s, register names, raw `asm { bfi }`, float as `0x457ff000` |
| Session limit | none | free version: 30-minute sessions |

Ghidra's output is materially better for spec-writing (it recovers the MK3 pad report layout:
3 bytes per pad, 4-bit event type, 12-bit pressure). Its one quirk here: it dropped the implicit
`this`, so parameter names are shifted by one.

## Full main binary (286 MB arm64 slice) — measured

| | Ghidra 12.1.4 via REA CLI | Hopper (licensed), `-a` + save script |
|---|---|---|
| Full analysis | **3 h 01 m** (10,869 s), peak RSS **13.7 GB** (`GHIDRA_HEADLESS_MAXMEM=20G`) | **2 h 33 m** (9,171 s), peak RSS ~3.7–4.7 GB |
| Persisted result | None (REA deletes the project) | `analysis/maschine_arm64.hop`, 686 MB, saved in 88 s |
| Reopen | n/a | ~65 s in the GUI, ~11–13 GB RAM; not scriptable (see tooling gaps) |

Most of Ghidra's time went to `DecompilerSwitchAnalyzer`. Sample output for
`TransportAccess::jumpToSection` is 47 lines and readable: it looks up the section key and
posts an `NI::MASCOMMON::RT::SectionKeyEvent` through the `RTEventSender`'s `std::function`.
Watch for linker identical-code-folding: there `Song::getRegistryKey` shows up as
`SC3::Product::getRegistryKey`, so a symbol name on a folded function can name the wrong class.

Hopper is now licensed (no 30-minute limit, can save), so the "free version" rows above no
longer apply. Recommendation: Ghidra for decompiling, once a persistent project exists; until
then, use the 11 MB agent binaries or targeted reads with our own tools.

## UI: what exists and what a rebuild needs

- The desktop UI is mostly hand-written Qt Widgets C++: 452 classes in
  `NI::MASCHINE::GUI` (main menu, header, arranger, pattern editor, mixer channel strips,
  browser, preference dialogs). Layout and painting live in code, not in data files.
- QML covers device panels only: effects, drum synths, Sampler, PolySynth, plug-in host,
  plus parts of the pattern editor and group panel (~95 files).
- Hardware screens are separate: NWL stylesheets and Lua (already extracted).

So a faithful copy of the desktop UI isn't close. Status:

1. **Done:** `apps/viewer` shows any local project read-only: arranger, pattern grid and
   piano roll.
2. A per-panel spec: `out/gui/bindings.md` lists what each widget reads and edits. Plus
   the user-visible strings, plus observing the running app, that is enough to design
   our own UI in our own words, as the clean-room boundary requires.
