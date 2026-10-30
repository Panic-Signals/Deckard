# How Maschine 3's audio engine talks to the rest of the app

These findings come from the arm64 binary: the symbol table (`out/decomp/nm-text.tsv`),
Ghidra decompiles of `ni::ae::EventQueue<…>::push` / `insertIntoBuffer`,
`ni::ae::ProcessingEngine::{process, processSlice, compile, getSecondsToFadeOnCompile}`
and `ThreadedGraphExecutor::process`, and `objdump` of every call site of `push`.

## Events from the UI and controllers: a lock-free queue, applied at the next block

- `ni::ae::EventQueue<MidiEvent, …RT::NoteEvent, SceneEvent, GuiKnobEvent, EncoderEvent,
  GroupButtonEvent, KillAllMidiNotesEvent, LoopEvent, SnapshotEvent, TouchstripEvent, …>`
  wraps a `boost::lockfree::stack<pair<unsigned long, variant<…>>>`, which is
  fixed-size (pre-allocated nodes).
- `push(event, time)` pushes onto the stack. When the stack is full, it counts a
  dropped event (a counter at +200) and does not block. [exact]
- The audio side drains the stack. `insertIntoBuffer` turns each `(time, event)` into
  a `TimedIntrusiveEvent` from an `ObjectPool`, a pre-allocated pool, and puts it into a
  `boost::intrusive::multiset` ordered by time. [exact]
- **Every call site passes time 0** (17 sites, checked in the disassembly):
  - the controller hardware thread (`KompleteControllerBase::onSwitchEventNHLThread`,
    `onEncoderEventNHLThread`, `MaschineControllerBase::onPedalEventNHLThread`);
  - GUI knob drags (`ParameterDraggerModulated::onRing*`);
  - `ProcessingAccess::pushEvent`, `IdeaSpaceAccess::createOrRequestFocusScene`,
    `MaschineAccess::chokeAll`, `TransportAccess::sendLoopStateUpdate`, and parameter
    snapshots.

  So UI and controller actions take effect at the start of the next audio block. They
  are not placed at the moment they happened within it. [exact]
- Other lock-free stacks:
  - `NI::AB::MidiEvent` (capacity 500): MIDI;
  - `HOSTING::ParameterChange` / `InputParameterChange`: plug-in parameters;
  - `MasterClockLink::TransportStateChange` (128): Link transport;
  - `HW::ControllerEvent`, `KeyIlluminationEvent`: hardware;
  - `RecordedEvents` variants: events recorded back to the document.
- Values for the UI go out through `boost::lockfree::queue<perfdata::TimedValue<float>>`
  (capacity 10000) and similar paths. The GUI never reads the engine directly. [inferred
  from types]

## Project changes: compile a new graph, hand it over, crossfade

- `ProcessingEngine::compile(SignalGraph&)` runs off the audio thread. It builds a
  `CompilationImpl` (processing graph, depths, output map) and compares it with the
  running one (`getProcessorsWithChangedRouting`, `compareCompilations`). [exact]
- The handover is double-buffered. A byte at +0xa0 selects one of two prepared slots,
  and `processSlice` reads it at the start of each slice. `compile` then waits on a
  dispatch semaphore (100 ms timeout) for the audio thread to take the new graph;
  `processSlice` signals it. Only if the audio thread isn't running does `compile` flip
  the index itself, under a mutex. [exact control flow; field meanings inferred]
- `getSecondsToFadeOnCompile()` = a frame count ÷ sample rate. Graph changes crossfade
  (`FadeUtils::ContinuousFade`). [exact formula; use inferred]

## The audio callback

- `ProcessingEngine::process()` does `try_lock` on a mutex. If it fails, the whole block
  is skipped. [exact] The only thing that takes that mutex is the not-running fallback
  above, so during playback it is never contended.
- The block is processed in slices (`setFramesPerSlice`, `getMaxNumFramesNextSlice`,
  `prepareNextSlice`).
- The graph renders on several cores: `ThreadedGraphExecutor` with a
  `WorkerThreadPool`, `RenderJob`s with input and output dependencies, and
  `setMaxNumberOfProcessingThreads` / `setWorkgroup` (macOS audio workgroups).

## What it means for DECKARD

- A message queue doesn't cost Maschine any timing accuracy. It applies UI and
  controller events at the next block, which is what DECKARD's locked calls do too.
  Sequenced notes stay sample-accurate inside the engine in both.
- The differences that matter for big projects:
  1. edits are handed over without the audio thread ever waiting;
  2. graph changes crossfade;
  3. rendering is spread across cores.

## Done in DECKARD

- **Multi-core rendering** (`crates/engine/src/pool.rs`, `Engine::render_pass`):
  - Each sound's work (its Sampler voices or plug-in, then its effects) is a job.
    Jobs run on a pool of worker threads plus the audio thread. The workers run at
    time-constraint priority and join the output device's audio workgroup.
  - Side-chain feeds, mixing, recording taps and meters then run on the audio thread
    in a fixed order, so the output is bit-identical whatever the thread count
    (`worker_threads_render_the_same`; real projects checked with
    `deckard-player --threads N`).
  - A side-chain listener that also feeds a side-chain runs its pass one sound at a
    time.
  - The switch is Preferences ▸ Audio ▸ Multi-core.
- **Handing edits over without waiting, and crossfading changes:** done, and taken
  further (sample-accurate live input, click-free level and effect changes); see
  docs/engine-design.md.
