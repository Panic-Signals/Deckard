# Hosting Audio Units on macOS

## Why

Deckard should match Maschine on plug-ins, and Maschine hosts Audio Units on macOS.
Today Deckard lists installed AUs in the "+" menu but greys them out: it can't load one.
The plug-in that prompted this is Output's Portal (`aufx PRTL Outp`), but the goal is the
format, not that one plug-in.

**Linux and Windows are out.** Audio Units are an Apple technology and neither system has
them. All the AU code is `cfg(target_os = "macos")`; the other builds must keep compiling
and behave as they do now.

## Goals

An AU works in Deckard the way a VST3 does now:

1. **Add it.** AUs in the "+" menu (the "Audio Units" list, by vendor) can be picked for a
   sound's instrument slot or any chain slot. Instruments (`aumu`) and effects (`aufx`) work
   fully. MIDI-controlled effects (`aumf`) run as effects, without notes (see Non-goals).
2. **Hear it.** It processes audio, plays notes, takes MIDI controllers and pitch bend, and
   follows tempo and transport (Portal has tempo-synced settings).
3. **See it.** Its own window opens from the same place as a VST3's, and resizes.
4. **Control it.** Its parameters appear on the plug-in pages and the controller, read with
   the plug-in's own value text, and can be automapped and modulated.
5. **Save it.** Its settings survive save and reopen, undo, and engine rebuilds, and an edit
   in its window marks the project as changed.
6. **Stay compatible with Maschine.** A Maschine project using an AU opens in Deckard with
   its settings, and a project Deckard saves opens in Maschine with its settings.

## Non-goals

These are my calls, not requests; tell me if any should be in.

- Loading a different format of the same plug-in in place of a missing AU.
- The Presets submenu for plug-in presets (open in `docs/PARITY.md` for every format).
- Validating plug-ins in a separate process when scanning, as Maschine does. Deckard doesn't
  do this for VST3 either, so a bad AU can take the app down the way a bad VST3 can.
- An AUv3-specific code path. AUs load the way Maschine loads them (AU v2 calls, section 3).
  An AUv3 the system can bridge will work; one it can't shows the usual load error.
- Anything beyond what VST3 hosting does now, such as new routing features. Two results:
  - **`aumf` gets no notes yet.** The engine sends notes only to a sound's instrument, for
    every format. An `aumf` processes audio in an effect slot, and its id is saved as `MF`.
    Sending it notes is engine routing work of its own.
  - **Extra outputs aren't audible.** `output_bus` works as for VST3, but the engine only
    plays the main output, for every format.

## Anti-goals

- The audio thread must not start waiting on the UI. AU hosting must not add allocation,
  locks or Objective-C calls to the render path beyond what the plug-in itself does.
- A project must never lose an AU's settings because it was opened where the AU can't load
  (missing plug-in, Linux, Windows). The id, name and chunk stay as they were, as they do for
  a missing VST3.
- VST3 and Deckard's own plug-ins must not change behaviour.

## What Maschine does (the format to match)

Full detail is in `docs/plugins-hosting.md`, sections 1.1 to 1.3. The parts that matter:

- **Id:** `<vendor>:AU:<INSTR|FX|MF>:<uid>`. `uid` is the component's subtype four-character
  code read big-endian as a signed 32-bit integer and printed in decimal. Portal's would be
  `Output:AU:FX:1347572812`. The manufacturer code and component type are not in the uid.
- **Vendor and name:** split from the component's name, `"Vendor: Plugin"`. `scan.rs` already
  does this.
- **Chunk:** the usual `[version 1][chunkType][size][data]` header, little-endian.
  - Non-NI AU: type 3, data is the AU's `kAudioUnitProperty_ClassInfo` as an XML plist.
  - NI AU: type 1, data is only the raw bytes of the ClassInfo key `vstdata`.
  - Loading type 1 or 4: build a ClassInfo dictionary (`version` 0; `type`, `subtype`,
    `manufacturer` as SInt32 numbers; `vstdata` the data) and set that.
  - Types 6 and 7 are NI's private round-trip states: the plug-in stays at its defaults,
    the chunk is kept, and it's replaced only by a real edit. Never written.
  - After setting a preset, send `AUParameterListenerNotify` for any parameter.
- **Parameter ids** saved on plug-in pages are `AudioUnitParameterID`s.

## How it fits in

### Plug-in layer (`crates/plugins`)

- New module `au.rs` (macOS only) with a running-AU type implementing the same operations as
  `Vst3`. `Kind` gains `AudioUnit(Box<..>)`, and the `each!` macro carries it, so the rest of
  the app keeps calling `Plugin`.
- `PluginId::parse` accepts `AU` everywhere. `id.format` is `Format::AudioUnit`. `cid` holds
  `"AU"`, type, subtype and a hash of the vendor, so instance reuse in `Host::instance` keeps
  working and two vendors with one subtype stay apart.
- On other systems `Host::instance` refuses `AU` with the "format not supported" message.
- `Host::instance` gets an AU branch: main thread only, like VST3; no `Module`. The component
  is found with `AudioComponentFindNext` by type and subtype, then matched on the vendor name
  from the id. If no vendor matches and exactly one component has that type and subtype, use
  it. A clear "not installed" error otherwise.
- `state.rs`: `unwrap_chunk` and `wrap_chunk` need the chunk type for AU (they always write
  type 1 and ignore it on read today). A chunk that hasn't been edited is kept as loaded, as
  VST3's is.

### Running an AU (`au.rs`)

- Instantiate with `AudioComponentInstanceNew`, using the calls in
  `docs/plugins-hosting.md` section 1.3.
- Before `AudioUnitInitialize`: set sample rate, `MaximumFramesPerSlice` to `MAX_BLOCK`, and a
  non-interleaved Float32 stereo stream format. If stereo is refused, try mono and map it to
  the host's stereo; if nothing works, fail with an error naming the plug-in.
- Effects get their input through `kAudioUnitProperty_SetRenderCallback`, reading the pointers
  `process_input` was given. Render with `AudioUnitRender` into buffers Deckard owns, and read
  the returned `mData` pointers, since an AU may substitute its own.
- `kAudioUnitProperty_HostCallbacks` answers from the `Transport` for this block (tempo, beat,
  time signature, playing, position). Sample time in the timestamp advances monotonically.
- `MusicDeviceMIDIEvent` carries notes, CC and pitch bend. `takes_controller` is true for
  anything that accepts MIDI.
- Additional output elements map to `output_bus`, as with VST3 multi-output.

### Parameters

- Read `kAudioUnitProperty_ParameterList` and `ParameterInfo`. Normalize values linearly
  between min and max at the `Plugin` API (0..1 everywhere else in the app).
- Text comes from `ParameterStringFromValue` and `ParameterValueStrings`, then falls back to
  the number and unit.
- `parameter_list(true)` keeps what's readable and writable, not "non-automatable" if flagged.
- `Plugin::params()` is a VST3 handle today. It needs an AU form, so the UI can read and set
  parameters without taking the instance's lock. `AudioUnitGetParameter` and
  `AudioUnitSetParameter` are designed for that.
- Edits in the plug-in's own window reach `take_dirty` through an AU event listener on
  parameter changes and on the `ClassInfo` property.

### Modulation

An AU has one value per parameter, where VST3 keeps the knob in the controller and the
modulated value in the processor. So `modulate` calls `AudioUnitSetParameter` at the block
start and remembers the knob's value, and two things need care:

- **Saving:** `save_chunk` puts the knob values back before reading ClassInfo, then restores
  the modulated ones, so modulation never gets saved into the project.
- **The window:** the plug-in's own knob may show modulation. That's accepted.
- **Deckard's knobs** show the remembered knob value, so they don't move with modulation.

### Editor (`editor.rs`)

- `Editor` today assumes an `IPlugView`. It needs to hold either that or an AU view.
- Get the view from `kAudioUnitProperty_CocoaUI`: load the bundle, instantiate the class,
  call `uiViewForAudioUnit:withSize:`, and set it as the window's content. With no Cocoa UI,
  use `AUGenericView` from CoreAudioKit.
- `Editor` becomes an enum of the VST3 editor (unchanged) and the AU one.
- Size: `Editor::tick` compares the view's frame with the window and adjusts the window, in
  keeping with the existing rule that resizes happen on the tick, outside plug-in calls.
- AppKit calls stay on the main thread.

### App (`apps/viewer`)

- `Entry::hostable()` returns true for AU on macOS. The Plug-ins preferences note becomes
  VST2 only. "Use Audio Units" works as it does now.
- Choosing an AU from the menu builds its id per the format above. `aumf` gets `MF`, and
  every check for `:FX:` also takes `:MF:`, so it counts as an effect.
- `docs/PARITY.md` and `docs/plugins-hosting.md` are updated.

## Testing

**Automated, in CI on macOS.** Apple's own AUs are always installed, so a test can rely on
them. A `harness = false` test in `crates/plugins/tests/`, macOS only, like `vst3_state`:

- `AUDelay` (`aufx dely appl`): input reaches the output delayed; a parameter changed through
  the API reads back with its text; a saved chunk loaded into a new instance restores it; the
  chunk is type 3 and parses as XML plist.
- An Apple instrument (`aumu`): a note makes sound, and all-notes-off ends it. Which one is
  the builder's call, since `DLSMusicDevice` depends on a sound bank being present.
- Modulation: the saved chunk holds the knob value, not the modulated one.

Plus plain unit tests for id parsing and formatting and for chunk wrapping, which don't need
macOS. Targeted runs only while working; the full suite once at the end of the branch.

**By hand on this Mac:**

- Portal: add it, hear it, open its window, automate a knob, save, reopen.
- One NI AU (for example `Ni$Q` Raum): the type 1 chunk path.
- A Maschine project using an AU opens in Deckard with its settings; the same project saved
  from Deckard opens in Maschine with its settings.

## Left to the builder

- The binding approach for AudioToolbox and CoreAudioKit (hand-written `extern` blocks as
  `scan.rs` uses, or a crate).
- Telling an NI AU when saving: manufacturer `-NI-` with `vstdata` in its ClassInfo. Raum
  saves type 1 this way. Maschine's own flag isn't fully pinned down.
- What to do with parameters with a logarithmic display flag, as long as 0..1 stays linear
  across the stored range and the text is right.
- Whether undo needs anything beyond what the chunk path gives it.
- The order of work, as long as each step leaves VST3 hosting untouched.
