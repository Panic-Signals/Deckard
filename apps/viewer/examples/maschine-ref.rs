//! Reference renderer: run Maschine 3's own engine (its VST3 plug-in, `Maschine 3.vst3`)
//! on a project, to null-test DECKARD's ports of Maschine's internal effects.
//!
//! The plug-in's component state is a whole `.mxprj` file (the same `hsin` container,
//! byte for byte what Maschine saves), so a project is loaded by `setState` with the
//! file's bytes. See docs/maschine-reference.md.
//!
//! ```text
//! cargo run --release -p deckard --example maschine-ref -- <command> ...
//!
//! info                               buses and classes of Maschine 3.vst3
//! state <out.mxprj>                  the plug-in's default state (an empty project)
//! list <project.mxprj>               groups, sounds and their chains (with module params)
//! make <src.mxprj> <out.mxprj> <g> <s> [keep-slot|-] [field=value ...]
//!     a test project from <src>: every other sound emptied; sound g/s keeps only slot
//!     `keep-slot` (`-`: no slot, a pass-through), takes In 1 L+R as audio input
//!     (gain 0 dB), level 0 dB, pan centre, and goes straight to Ext. 2 (bypassing
//!     group and master); `field=value` set the kept module's parameters.
//!     Side-chain options (the kept slot's side-chain source):
//!     --sc G/S          sound G/S is the source: emptied (a pass-through), fed by
//!                       --sc-in, level --sc-level, out to --sc-out
//!     --sc-group        the source is G's group instead (the sound feeds it)
//!     --sc-in N         the source sound's audio input Ext(N) (default 1 = In 2 L+R)
//!     --sc-out none|parent|master|ext:N   its output (default none)
//!     --sc-level V      its stored level (default 0.75 = 0 dB); --sc-pan P its pan
//!     --sc-group-level V  the stored level of its group
//! roundtrip <in.mxprj> <out.mxprj>  load a project into Maschine and save what it holds
//! plant <src.mxprj> <out.mxprj> <g> <s>
//!     put a fake DECKARD plug-in (unknown to Maschine) on sound g/s, to check with
//!     `roundtrip` that Maschine keeps a missing plug-in's state
//! compare <a.wav> <b.wav> [max-lag]   null test: residual of b - a at the best lag
//! render <project.mxprj> <out.wav> [options]
//!     --input sweep|noise|sine:HZ|impulse|wav:PATH|none   (default sweep; -12 dBFS)
//!     --level DBFS        test signal level (default -12)
//!     --note N[:VEL]      MIDI note on at the start (off 0.5 s before the end)
//!     --notes T:N:V:D,... notes: at T seconds, key N, velocity V, D seconds long
//!     --cc T:NUM:VAL,...  controller changes      --bend T:VAL,...  pitch bend (0..16383)
//!                         (events land at the start of the 1024-frame host block
//!                         holding them; synth-null does the same)
//!     --seconds S         length (default 4)       --sr HZ (default 48000)
//!     --bus N             output bus (0 = Maschine Out 1 = master; default: 1 if the
//!                         project routes a sound to Ext. 2, else 0)
//!     --preroll S         silence processed before (default 1, lets samples load)
//!     --input-wav PATH    also write the test signal
//!     --input2 KIND       a signal on Maschine In 2 too (as --input; with
//!                         --level2 DBFS, default -12); --input2-wav PATH writes it
//!     --play [--tempo BPM] run the host transport from bar 1 (Maschine follows it and
//!                         plays the project's patterns)
//! ```
//!
//! The plug-in gets a scratch home (`$TMPDIR/maschine-ref-home`, or `MASCHINE_REF_HOME`;
//! `MASCHINE_REF_HOME=user` for the real one), so it leaves the user's Maschine settings,
//! log and content database alone.
//!
//! Plug-ins must be created on the main thread, and NI's crash in exit-time
//! destructors, so this ends with `deckard_plugins::exit`.

// Maschine's plug-in here is the macOS one (CoreFoundation and the bundle loader).
#[cfg(target_os = "macos")]
#[path = "maschine-ref/imp.rs"]
mod imp;

fn main() {
    #[cfg(target_os = "macos")]
    imp::main();
    #[cfg(not(target_os = "macos"))]
    eprintln!("maschine-ref runs Maschine's macOS plug-in: macOS only");
}
