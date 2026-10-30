// The reference renderer itself (see ../maschine-ref.rs): macOS only, as it loads
// Maschine's macOS plug-in.

use deckard_doc::{Document, SoundRoute, Target};
use deckard_model::{AudioDest, AudioSource, ScSource};
use deckard_plugins::module::Module;
use deckard_plugins::state::{Format, PluginId, State};
use deckard_plugins::{MAX_BLOCK, Plugin, Transport};
use mxprj::decode::Value;
use std::path::Path;
use std::sync::Arc;
use vst3::Steinberg::Vst::*;

const BUNDLE: &str = "/Library/Audio/Plug-Ins/VST3/Maschine 3.vst3";

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const std::ffi::c_void;
    fn CFRunLoopRunInMode(mode: *const std::ffi::c_void, seconds: f64, return_after_source_handled: u8) -> i32;
}

/// Let the plug-in's main-thread work run (its deferred tasks, timers) for `seconds`.
fn pump(seconds: f64) {
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, 0) };
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("maschine-ref: {msg}");
    deckard_plugins::exit(1)
}

fn utf16(b: &[u16]) -> String {
    String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])
}

/// A Maschine 3 instance with no state set.
fn maschine(sr: f64) -> Plugin {
    let module = Arc::new(Module::load(Path::new(BUNDLE)).unwrap_or_else(|e| die(e)));
    let class = module.classes.iter().find(|c| c.category == "Audio Module Class").unwrap_or_else(|| die("no audio class"));
    let id = PluginId { vendor: "Native Instruments".into(), format: Format::Vst3, kind: "INSTR".into(), cid: class.cid };
    Plugin::new(&module, id, &class.name, sr).unwrap_or_else(|e| die(e))
}

fn info() {
    let module = Arc::new(Module::load(Path::new(BUNDLE)).unwrap_or_else(|e| die(e)));
    for c in &module.classes {
        println!("class {:?} [{}] {}", c.name, c.category, c.cid.iter().map(|b| format!("{b:02X}")).collect::<String>());
    }
    let p = maschine(48_000.0);
    let comp = p.component().unwrap();
    for (mt, mn) in [(MediaTypes_::kAudio, "audio"), (MediaTypes_::kEvent, "event")] {
        for (d, dn) in [(BusDirections_::kInput, "in"), (BusDirections_::kOutput, "out")] {
            for i in 0..unsafe { comp.getBusCount(mt as i32, d as i32) } {
                let mut b: BusInfo = unsafe { std::mem::zeroed() };
                unsafe { comp.getBusInfo(mt as i32, d as i32, i, &mut b) };
                println!("{mn} {dn} {i}: {:?} channels={} main={}", utf16(&b.name), b.channelCount, b.busType == 0);
            }
        }
    }
    let params = p.parameters();
    println!("parameters: {}", params.len());
    for x in &params {
        println!("  {} {:?} steps {} default {}", x.id, x.title, x.steps, x.default);
    }
}

fn list(path: &str) {
    let d = Document::open(Path::new(path)).unwrap_or_else(|e| die(e));
    for (gi, g) in d.project().groups.iter().enumerate() {
        let c = &g.channel;
        println!(
            "group {gi} {:?} ({} slots) key mode {} midi {:?} ch {} start {}",
            g.name,
            g.slots.len(),
            c.key_mode,
            c.midi_source,
            c.midi_channel,
            c.start_note
        );
        for (si, s) in g.sounds.iter().enumerate() {
            if s.slots.is_empty() {
                continue;
            }
            let c = &s.channel;
            println!(
                "  sound {gi}/{si} {:?} in={:?} out={:?} midi {:?} ch {} base key {}",
                s.name, c.audio_in, c.dest, c.midi_source, c.midi_channel, c.base_key
            );
            for (k, slot) in s.slots.iter().enumerate() {
                println!("    slot {k}: {} {:?}{}", slot.module, slot.name, if slot.active { "" } else { " (bypassed)" });
                if let Some(pr) = &slot.plugin {
                    let hash = pr.chunk.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
                    println!("      plugin {} chunk {} bytes, fnv {hash:016x}", pr.id, pr.chunk.len());
                }
                for (f, v) in &slot.params {
                    println!("      {f} = {v}");
                }
            }
        }
    }
}

fn make(args: &[String]) {
    let [src, out, g, s, rest @ ..] = args else { die("make <src.mxprj> <out.mxprj> <g> <s> [keep-slot|-] [field=value ...]") };
    let (g, s): (usize, usize) = (g.parse().unwrap_or_else(|_| die("group")), s.parse().unwrap_or_else(|_| die("sound")));
    // Side-chain options, out of the positional arguments.
    let mut sc: Option<(usize, usize)> = None;
    let (mut sc_group, mut sc_in, mut sc_out, mut sc_level) = (false, 1i64, AudioDest::None, 0.75f64);
    let (mut sc_pan, mut sc_group_level) = (0.0f64, None::<f64>);
    // A Perform FX in the sound's group's chain instead of a kept slot: its mode.
    let mut perform: Option<f64> = None;
    // --host ID=field: host automation slot ID moves that Perform FX parameter.
    let mut host_slots: Vec<String> = Vec::new();
    let mut plain = Vec::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| die(format!("{a} needs a value")));
        match a.as_str() {
            "--sc" => {
                let v = val();
                let (x, y) = v.split_once('/').unwrap_or_else(|| die("--sc G/S"));
                sc = Some((x.parse().unwrap_or_else(|_| die("--sc G/S")), y.parse().unwrap_or_else(|_| die("--sc G/S"))));
            }
            "--sc-group" => sc_group = true,
            "--perform" => perform = Some(val().parse().unwrap_or_else(|_| die("--perform MODE (0-7)"))),
            "--host" => host_slots.push(val().clone()),
            "--sc-in" => sc_in = val().parse().unwrap_or_else(|_| die("--sc-in N")),
            "--sc-level" => sc_level = val().parse().unwrap_or_else(|_| die("--sc-level V")),
            "--sc-pan" => sc_pan = val().parse().unwrap_or_else(|_| die("--sc-pan P")),
            "--sc-group-level" => sc_group_level = Some(val().parse().unwrap_or_else(|_| die("--sc-group-level V"))),
            "--sc-out" => {
                let v = val();
                sc_out = match v.as_str() {
                    "none" => AudioDest::None,
                    "parent" => AudioDest::Parent,
                    "master" => AudioDest::Master,
                    x if x.starts_with("ext:") => AudioDest::Ext(x[4..].parse().unwrap_or_else(|_| die("--sc-out ext:N"))),
                    _ => die("--sc-out none|parent|master|ext:N"),
                }
            }
            _ => plain.push(a.clone()),
        }
    }
    let rest = &plain[..];
    let (keep, sets) = match rest.first() {
        Some(k) if !k.contains('=') => (if k == "-" { None } else { Some(k.parse::<usize>().unwrap_or_else(|_| die("slot"))) }, &rest[1..]),
        _ => (None, rest),
    };
    let mut d = Document::open(Path::new(src)).unwrap_or_else(|e| die(e));
    let groups: Vec<usize> = d.project().groups.iter().map(|g| g.sounds.len()).collect();
    if g >= groups.len() || s >= groups[g] {
        die("no such sound");
    }
    // Everything else silent: empty every other sound.
    for (gi, &n) in groups.iter().enumerate() {
        for si in 0..n {
            if (gi, si) != (g, s) && !d.project().groups[gi].sounds[si].slots.is_empty() {
                d.reset_sound(gi, si).unwrap_or_else(|e| die(format!("reset {gi}/{si}: {e}")));
            }
        }
    }
    let n = d.project().groups[g].sounds[s].slots.len();
    if let Some(k) = keep
        && k >= n
    {
        die(format!("sound {g}/{s} has {n} slots"));
    }
    for k in (0..n).rev() {
        if Some(k) != keep {
            d.remove_slot(g, s, k).unwrap_or_else(|e| die(format!("remove slot {k}: {e}")));
        }
    }
    if keep.is_some() {
        d.set_slot_active(g, s, 0, true).unwrap_or_else(|e| die(e));
    }
    let t = Target::Sound(g, s);
    d.set_sound_route(g, s, SoundRoute::AudioIn(AudioSource::Ext(0))).unwrap_or_else(|e| die(format!("audio in: {e}")));
    d.set_sound_route(g, s, SoundRoute::Output(AudioDest::Ext(1))).unwrap_or_else(|e| die(format!("output: {e}")));
    d.set("Gain", t, "input_gain", Value::Float(0.75)).unwrap_or_else(|e| die(format!("input gain: {e}")));
    d.set_level(t, 0.75).unwrap_or_else(|e| die(e));
    d.set_pan(t, 0.0).unwrap_or_else(|e| die(e));
    d.set_mute(t, false).unwrap_or_else(|e| die(e));
    // MIDI from the host on any channel, to this sound only (its group's keyboard off).
    // A plug-in instance reads its own MIDI source (`midi_source_plugin`), not the
    // standalone app's.
    let host = || "Host".to_string();
    d.set_sound_route(g, s, SoundRoute::MidiSource(host())).unwrap_or_else(|e| die(format!("MIDI source: {e}")));
    d.set_sound_route(g, s, SoundRoute::MidiSourcePlugin(host())).unwrap_or_else(|e| die(format!("plug-in MIDI source: {e}")));
    d.set("MIDI Channel", t, "midi_channel", Value::Int(0)).unwrap_or_else(|e| die(format!("MIDI channel: {e}")));
    // Older groups (before class version 0x12) have no Key Mode: their keyboard is off.
    if let Err(e) = d.set("Key Mode", Target::Group(g), "key_mode", Value::Int(0))
        && !e.contains("has no key_mode")
    {
        die(format!("key mode: {e}"));
    }
    if let Some((sg, ss)) = sc {
        if keep.is_none() {
            die("--sc needs a kept slot");
        }
        if sg >= groups.len() || ss >= groups[sg] || (sg, ss) == (g, s) {
            die("bad --sc sound");
        }
        let st = Target::Sound(sg, ss);
        for k in (0..d.project().groups[sg].sounds[ss].slots.len()).rev() {
            d.remove_slot(sg, ss, k).unwrap_or_else(|e| die(format!("sc remove slot {k}: {e}")));
        }
        d.set_sound_route(sg, ss, SoundRoute::AudioIn(AudioSource::Ext(sc_in))).unwrap_or_else(|e| die(format!("sc audio in: {e}")));
        d.set_sound_route(sg, ss, SoundRoute::Output(sc_out)).unwrap_or_else(|e| die(format!("sc output: {e}")));
        d.set("Gain", st, "input_gain", Value::Float(0.75)).unwrap_or_else(|e| die(format!("sc input gain: {e}")));
        d.set_level(st, sc_level).unwrap_or_else(|e| die(e));
        d.set_pan(st, sc_pan).unwrap_or_else(|e| die(e));
        d.set_mute(st, false).unwrap_or_else(|e| die(e));
        if let Some(v) = sc_group_level {
            d.set_level(Target::Group(sg), v).unwrap_or_else(|e| die(e));
        }
        let src = if sc_group { ScSource::Group(sg) } else { ScSource::Sound(sg, ss) };
        d.set_sidechain(g, s, 0, Some(src)).unwrap_or_else(|e| die(format!("side-chain: {e}")));
    }
    // --perform: the sound passes its input to its group, whose chain is one Perform FX
    // (engaged, in MODE), out of Ext. 2 at unity.
    if let Some(mode) = perform {
        use deckard_doc::Chain;
        if keep.is_some() {
            die("--perform takes no kept slot (use -)");
        }
        for k in (0..d.project().groups[g].slots.len()).rev() {
            d.remove_slot_in(Chain::Group(g), k).unwrap_or_else(|e| die(format!("group slot {k}: {e}")));
        }
        d.add_perform_fx(g).unwrap_or_else(|e| die(format!("perform fx: {e}")));
        let fx = Target::ChainModule(Chain::Group(g), 0);
        d.set("Param", fx, "mode", Value::Float(mode)).unwrap_or_else(|e| die(format!("mode: {e}")));
        d.set("Param", fx, "touchstrip_on_off", Value::Float(1.0)).unwrap_or_else(|e| die(format!("engage: {e}")));
        d.set_sound_route(g, s, SoundRoute::Output(AudioDest::Parent)).unwrap_or_else(|e| die(format!("output: {e}")));
        d.set_group_route(g, None, AudioDest::Ext(1)).unwrap_or_else(|e| die(format!("group output: {e}")));
        let gt = Target::Group(g);
        d.set_level(gt, 0.75).unwrap_or_else(|e| die(e));
        d.set_pan(gt, 0.0).unwrap_or_else(|e| die(e));
        d.set_mute(gt, false).unwrap_or_else(|e| die(e));
    }
    for hs in &host_slots {
        let (id, f) = hs.split_once('=').unwrap_or_else(|| die("--host ID=field"));
        let id: usize = id.parse().unwrap_or_else(|_| die("--host ID=field"));
        if perform.is_none() {
            die("--host needs --perform");
        }
        let t = Target::ChainModule(deckard_doc::Chain::Group(g), 0);
        d.set_host_automation(id, Some((t, f))).unwrap_or_else(|e| die(format!("--host {hs}: {e}")));
    }
    for kv in sets {
        let (f, v) = kv.split_once('=').unwrap_or_else(|| die(format!("bad {kv}")));
        let v: f64 = v.parse().unwrap_or_else(|_| die(format!("bad value in {kv}")));
        let t = if perform.is_some() {
            Target::ChainModule(deckard_doc::Chain::Group(g), 0)
        } else if keep.is_some() {
            Target::Module(g, s, 0)
        } else {
            die("no module to set parameters of")
        };
        d.set("Param", t, f, Value::Float(v)).unwrap_or_else(|e| die(format!("{f}: {e}")));
    }
    d.save_as(Path::new(out)).unwrap_or_else(|e| die(e));
    let snd = &d.project().groups[g].sounds[s];
    println!(
        "wrote {out}: sound {g}/{s} {:?}, in={:?} out={:?} side-chain {:?}",
        snd.name, snd.channel.audio_in, snd.channel.dest, snd.channel.sidechain
    );
    for slot in &snd.slots {
        println!("  {} {:?}: {:?}", slot.module, slot.name, slot.params);
    }
    if perform.is_some() {
        let gr = &d.project().groups[g];
        println!("group {g} out={:?}", gr.channel.dest);
        for slot in &gr.slots {
            println!("  {}: {:?}", slot.module, slot.params);
        }
    }
}

/// Deterministic white noise in -1..1.
fn noise(n: usize) -> Vec<f32> {
    let mut x: u32 = 0x1234_5678;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
        })
        .collect()
}

fn signal(kind: &str, n: usize, sr: f64, gain: f32) -> (Vec<f32>, Vec<f32>) {
    let mono = |v: Vec<f32>| (v.clone(), v);
    let (l, r) = match kind {
        "none" => mono(vec![0.0; n]),
        "noise" => mono(noise(n)),
        "impulse" => {
            let mut v = vec![0.0; n];
            v[0] = 1.0;
            mono(v)
        }
        "sweep" => {
            // Exponential sine sweep 20 Hz .. 20 kHz over the whole length.
            let (f0, f1, t) = (20.0f64, 20_000.0f64, n as f64 / sr);
            let k = (f1 / f0).ln();
            mono((0..n).map(|i| (2.0 * std::f64::consts::PI * f0 * t / k * (((i as f64 / sr) / t * k).exp() - 1.0)).sin() as f32).collect())
        }
        k if k.starts_with("sine:") => {
            let f: f64 = k[5..].parse().unwrap_or_else(|_| die("sine:HZ"));
            mono((0..n).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / sr).sin() as f32).collect())
        }
        k if k.starts_with("wav:") => {
            let mut r = hound::WavReader::open(&k[4..]).unwrap_or_else(|e| die(e));
            let spec = r.spec();
            let v: Vec<f32> = match spec.sample_format {
                hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
                hound::SampleFormat::Int => {
                    let s = (1u64 << (spec.bits_per_sample - 1)) as f32;
                    r.samples::<i32>().map(|x| x.unwrap() as f32 / s).collect()
                }
            };
            let ch = spec.channels as usize;
            let get = |c: usize| -> Vec<f32> {
                let mut o: Vec<f32> = v.chunks(ch).map(|f| f[c.min(ch - 1)]).collect();
                o.resize(n, 0.0);
                o
            };
            // Played as is (no level change).
            return (get(0), get(1));
        }
        _ => die(format!("unknown input {kind}")),
    };
    (l.into_iter().map(|x| x * gain).collect(), r.into_iter().map(|x| x * gain).collect())
}

fn write_wav(path: &str, sr: u32, l: &[f32], r: &[f32]) {
    let spec = hound::WavSpec { channels: 2, sample_rate: sr, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(path, spec).unwrap_or_else(|e| die(e));
    for (a, b) in l.iter().zip(r) {
        w.write_sample(*a).unwrap();
        w.write_sample(*b).unwrap();
    }
    w.finalize().unwrap();
}

fn rms_db(x: &[f32]) -> f64 {
    let r = (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt();
    20.0 * r.max(1e-12).log10()
}

/// The plug-in driven directly (`IAudioProcessor::process`) with our own buffers, so
/// that every input bus can carry a signal (`Plugin::process_input` feeds only In 1).
struct Direct {
    processor: vst3::ComPtr<IAudioProcessor>,
    ins: Vec<[Vec<f32>; 2]>,
    outs: Vec<[Vec<f32>; 2]>,
    sr: f64,
    clock: i64,
}

impl Direct {
    fn new(p: &Plugin, sr: f64) -> Direct {
        use vst3::ComRef;
        let comp: ComRef<'_, vst3::Steinberg::Vst::IComponent> = p.component().unwrap().as_com_ref();
        let processor = comp.cast::<IAudioProcessor>().unwrap_or_else(|| die("no audio processor"));
        let count = |d| unsafe { p.component().unwrap().getBusCount(MediaTypes_::kAudio as i32, d as i32) } as usize;
        let bufs = |n| (0..n).map(|_| [vec![0f32; MAX_BLOCK], vec![0f32; MAX_BLOCK]]).collect();
        Direct { processor, ins: bufs(count(BusDirections_::kInput)), outs: bufs(count(BusDirections_::kOutput)), sr, clock: 0 }
    }

    /// Process `k` frames with `inputs[bus]` = (left, right) on the input buses given.
    fn process(&mut self, k: usize, t: &Transport, inputs: &[Option<(&[f32], &[f32])>]) {
        let mut silent = vec![u64::MAX; self.ins.len()];
        for (b, ch) in self.ins.iter_mut().enumerate() {
            match inputs.get(b).copied().flatten() {
                Some((l, r)) => {
                    ch[0][..k].copy_from_slice(&l[..k]);
                    ch[1][..k].copy_from_slice(&r[..k]);
                    silent[b] = 0;
                }
                None => {
                    ch[0][..k].fill(0.0);
                    ch[1][..k].fill(0.0);
                }
            }
        }
        let mut in_ptrs: Vec<[*mut f32; 2]> = self.ins.iter_mut().map(|[l, r]| [l.as_mut_ptr(), r.as_mut_ptr()]).collect();
        let mut out_ptrs: Vec<[*mut f32; 2]> = self.outs.iter_mut().map(|[l, r]| [l.as_mut_ptr(), r.as_mut_ptr()]).collect();
        let bus = |p: &mut [*mut f32; 2], silence: u64| AudioBusBuffers {
            numChannels: 2,
            silenceFlags: silence,
            __field0: AudioBusBuffers__type0 { channelBuffers32: p.as_mut_ptr() },
        };
        let mut in_buses: Vec<AudioBusBuffers> = in_ptrs.iter_mut().zip(&silent).map(|(p, s)| bus(p, *s)).collect();
        let mut out_buses: Vec<AudioBusBuffers> = out_ptrs.iter_mut().map(|p| bus(p, 0)).collect();
        let mut ctx: ProcessContext = unsafe { std::mem::zeroed() };
        {
            use ProcessContext_::StatesAndFlags_::*;
            ctx.state = kTempoValid | kTimeSigValid | kProjectTimeMusicValid | kContTimeValid | if t.playing { kPlaying } else { 0 };
        }
        ctx.sampleRate = self.sr;
        ctx.projectTimeSamples = self.clock;
        ctx.continousTimeSamples = self.clock;
        ctx.projectTimeMusic = t.quarters;
        ctx.tempo = t.tempo;
        ctx.timeSigNumerator = 4;
        ctx.timeSigDenominator = 4;
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: k as i32,
            numInputs: in_buses.len() as i32,
            numOutputs: out_buses.len() as i32,
            inputs: in_buses.as_mut_ptr(),
            outputs: out_buses.as_mut_ptr(),
            inputParameterChanges: std::ptr::null_mut(),
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: &mut ctx,
        };
        unsafe { self.processor.process(&mut data) };
        self.clock += k as i64;
    }

    fn output_bus(&self, b: usize, k: usize) -> Option<(&[f32], &[f32])> {
        self.outs.get(b).map(|[l, r]| (&l[..k], &r[..k]))
    }
}

/// A MIDI event of `--notes`, `--cc` or `--bend`.
#[derive(Clone, Copy, Debug)]
enum Ev {
    On(i64, i64),
    Off(i64),
    Cc(u8, u8),
    Bend(u16),
}

/// The events of `--notes T:N:V:D,…`, `--cc T:NUM:VAL,…` and `--bend T:VAL,…`, at
/// frames rounded down to the 1024-frame host block holding them, in order (note-offs
/// before note-ons at the same frame).
fn events(notes: Option<&str>, cc: Option<&str>, bend: Option<&str>, sr: f64) -> Vec<(usize, Ev)> {
    let at = |t: &str| ((t.parse::<f64>().unwrap_or_else(|_| die(format!("bad time {t}"))) * sr) as usize) / MAX_BLOCK * MAX_BLOCK;
    let num = |x: &str| x.parse::<i64>().unwrap_or_else(|_| die(format!("bad number {x}")));
    let mut out = Vec::new();
    for item in notes.into_iter().flat_map(|s| s.split(',')) {
        let f: Vec<&str> = item.split(':').collect();
        let [t, n, v, d] = f[..] else { die("--notes T:N:V:D,...") };
        out.push((at(t), Ev::On(num(n), num(v))));
        let end = t.parse::<f64>().unwrap_or(0.0) + d.parse::<f64>().unwrap_or_else(|_| die("bad duration"));
        out.push((at(&end.to_string()), Ev::Off(num(n))));
    }
    for item in cc.into_iter().flat_map(|s| s.split(',')) {
        let f: Vec<&str> = item.split(':').collect();
        let [t, c, v] = f[..] else { die("--cc T:NUM:VAL,...") };
        out.push((at(t), Ev::Cc(num(c) as u8, num(v) as u8)));
    }
    for item in bend.into_iter().flat_map(|s| s.split(',')) {
        let f: Vec<&str> = item.split(':').collect();
        let [t, v] = f[..] else { die("--bend T:VAL,...") };
        out.push((at(t), Ev::Bend(num(v) as u16)));
    }
    out.sort_by_key(|(f, e)| (*f, !matches!(e, Ev::Off(_))));
    out
}

fn render(args: &[String]) {
    let [project, out, opts @ ..] = args else { die("render <project.mxprj> <out.wav> [options]") };
    let opt = |name: &str| opts.iter().position(|a| a == name).and_then(|i| opts.get(i + 1)).map(String::as_str);
    let sr: f64 = opt("--sr").map_or(48_000.0, |v| v.parse().unwrap_or_else(|_| die("--sr")));
    let seconds: f64 = opt("--seconds").map_or(4.0, |v| v.parse().unwrap_or_else(|_| die("--seconds")));
    let preroll: f64 = opt("--preroll").map_or(1.0, |v| v.parse().unwrap_or_else(|_| die("--preroll")));
    let level: f64 = opt("--level").map_or(-12.0, |v| v.parse().unwrap_or_else(|_| die("--level")));
    let input = opt("--input").unwrap_or(if opt("--note").is_some() || opt("--notes").is_some() { "none" } else { "sweep" });
    let note = opt("--note").map(|v| {
        let mut p = v.split(':');
        let n: i64 = p.next().unwrap().parse().unwrap_or_else(|_| die("--note"));
        let vel: i64 = p.next().map_or(100, |x| x.parse().unwrap_or_else(|_| die("--note vel")));
        (n, vel)
    });
    let bytes = std::fs::read(project).unwrap_or_else(|e| die(format!("{project}: {e}")));
    // Default bus: Ext. 2 if a sound or group goes there (as `make` routes it).
    let bus: usize = opt("--bus").map(|v| v.parse().unwrap_or_else(|_| die("--bus"))).unwrap_or_else(|| {
        let d = Document::from_bytes(bytes.clone(), None).ok();
        let direct = d.is_some_and(|d| {
            let p = d.project();
            p.groups.iter().flat_map(|g| &g.sounds).any(|s| s.channel.dest == AudioDest::Ext(1))
                || p.groups.iter().any(|g| g.channel.dest == AudioDest::Ext(1))
        });
        direct as usize
    });

    // Automation left pointing at a removed plug-in's parameter: Maschine fails to load
    // the project and then crashes cleaning up (until crates/doc removes such lanes).
    let has = |s: &str| bytes.windows(s.len()).any(|w| w == s.as_bytes());
    if has("NI::MASCOMMON::DATA::PluginParameter") && !has("NI::MASCHINE::DATA::PluginHost") {
        die("the project has automation of a plug-in it no longer holds (Maschine would crash loading it); pick another source");
    }
    let mut p = maschine(sr);
    p.set_state(&State { component: bytes, controller: Vec::new() }).unwrap_or_else(|e| die(e));
    let play = opts.iter().any(|a| a == "--play");
    let realtime = opts.iter().any(|a| a == "--realtime");
    let tempo: f64 = opt("--tempo").map_or(120.0, |v| v.parse().unwrap_or_else(|_| die("--tempo")));
    let mut t = Transport { tempo, numerator: 4, denominator: 4, playing: false, quarters: 0.0 };
    // Preroll: let it load (samples load in the background), in real time.
    let start = std::time::Instant::now();
    let pre = (preroll * sr) as usize;
    let mut done = 0;
    while done < pre {
        let k = MAX_BLOCK.min(pre - done);
        p.process(k, &t);
        done += k;
        pump(k as f64 / sr);
    }
    eprintln!("preroll {:.2}s took {:.2}s", preroll, start.elapsed().as_secs_f64());

    let n = (seconds * sr) as usize;
    let gain = 10f64.powf(level / 20.0) as f32;
    let (il, ir) = signal(input, n, sr, gain);
    let level2: f64 = opt("--level2").map_or(-12.0, |v| v.parse().unwrap_or_else(|_| die("--level2")));
    let in2 = opt("--input2").map(|k| signal(k, n, sr, 10f64.powf(level2 / 20.0) as f32));
    let mut direct = in2.as_ref().map(|_| {
        let mut d = Direct::new(&p, sr);
        d.clock = pre as i64;
        d
    });
    let (mut ol, mut or) = (Vec::with_capacity(n), Vec::with_capacity(n));
    if let Some((k, v)) = note {
        p.note_on(k, v);
    }
    let mut events = events(opt("--notes"), opt("--cc"), opt("--bend"), sr);
    events.reverse();
    let off_at = n.saturating_sub((0.5 * sr) as usize);
    // --then SECONDS PROJECT: load another state that far in (on a block boundary).
    let then = opts.iter().position(|a| a == "--then").map(|i| {
        let secs: f64 = opts.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or_else(|| die("--then SECONDS PROJECT"));
        let path = opts.get(i + 2).unwrap_or_else(|| die("--then SECONDS PROJECT"));
        let b = std::fs::read(path).unwrap_or_else(|e| die(format!("{path}: {e}")));
        ((secs * sr) as usize / MAX_BLOCK * MAX_BLOCK, b)
    });
    let mut then = then;
    // --param SECONDS ID VALUE (repeatable): a host parameter change at that time (the
    // block it falls in; block = 1024 frames).
    let mut params: Vec<(usize, u32, f64)> = opts
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--param")
        .map(|(i, _)| {
            let get = |k: usize| opts.get(i + k).unwrap_or_else(|| die("--param SECONDS ID VALUE"));
            let secs: f64 = get(1).parse().unwrap_or_else(|_| die("--param SECONDS"));
            (
                (secs * sr) as usize,
                get(2).parse().unwrap_or_else(|_| die("--param ID")),
                get(3).parse().unwrap_or_else(|_| die("--param VALUE")),
            )
        })
        .collect();
    params.sort_by_key(|p| std::cmp::Reverse(p.0));
    let mut energy = [0f64; 16];
    let mut at = 0;
    while at < n {
        if let Some((when, _)) = &then
            && at >= *when
        {
            let (_, b) = then.take().unwrap();
            p.set_state(&State { component: b, controller: Vec::new() }).unwrap_or_else(|e| die(e));
            eprintln!("state changed at {at}");
        }
        let k = MAX_BLOCK.min(n - at);
        while let Some(&(when, id, v)) = params.last()
            && when < at + k
        {
            params.pop();
            p.set_param(id, v);
        }
        if let Some((key, _)) = note
            && at <= off_at
            && off_at < at + k
        {
            p.note_off(key);
        }
        while let Some(&(f, e)) = events.last()
            && f < at + k
        {
            events.pop();
            match e {
                Ev::On(key, vel) => p.note_on(key, vel),
                Ev::Off(key) => p.note_off(key),
                Ev::Cc(num, val) => p.control_change(num, val),
                Ev::Bend(v) => p.pitch_bend(v),
            }
        }
        let feed = input != "none";
        t.playing = play;
        t.quarters = at as f64 / sr * tempo / 60.0;
        let (l, r) = if let (Some(dp), Some((jl, jr))) = (direct.as_mut(), in2.as_ref()) {
            dp.process(k, &t, &[feed.then(|| (&il[at..at + k], &ir[at..at + k])), Some((&jl[at..at + k], &jr[at..at + k]))]);
            for (b, e) in energy.iter_mut().enumerate() {
                if let Some((l, r)) = dp.output_bus(b, k) {
                    *e += l.iter().chain(r).map(|v| (*v as f64).powi(2)).sum::<f64>();
                }
            }
            dp.output_bus(bus, k).unwrap_or_else(|| die(format!("no output bus {bus}")))
        } else {
            p.process_input(k, &t, feed.then(|| (&il[at..at + k], &ir[at..at + k])));
            for (b, e) in energy.iter_mut().enumerate() {
                if let Some((l, r)) = p.output_bus(b, k) {
                    *e += l.iter().chain(r).map(|v| (*v as f64).powi(2)).sum::<f64>();
                }
            }
            p.output_bus(bus, k).unwrap_or_else(|| die(format!("no output bus {bus}")))
        };
        if realtime {
            pump(k as f64 / sr);
        }
        ol.extend_from_slice(l);
        or.extend_from_slice(r);
        at += k;
    }
    write_wav(out, sr as u32, &ol, &or);
    if let Some(path) = opt("--input-wav") {
        write_wav(path, sr as u32, &il, &ir);
    }
    if let (Some(path), Some((jl, jr))) = (opt("--input2-wav"), in2.as_ref()) {
        write_wav(path, sr as u32, jl, jr);
    }
    let active: Vec<String> = energy
        .iter()
        .enumerate()
        .filter(|(_, e)| **e > 1e-9)
        .map(|(b, e)| format!("{b}: {:.1} dB", 10.0 * (e / (2 * n) as f64).log10()))
        .collect();
    println!("buses with signal: {}", if active.is_empty() { "none".into() } else { active.join(", ") });
    let peak = ol.iter().chain(&or).fold(0f32, |a, v| a.max(v.abs()));
    println!(
        "wrote {out}: {n} frames at {sr} Hz from bus {bus}; input {input} {:.1} dB RMS, output L {:.1} / R {:.1} dB RMS, peak {:.4}",
        rms_db(&il),
        rms_db(&ol),
        rms_db(&or),
        peak
    );
}

fn read_wav(path: &str) -> (Vec<f32>, Vec<f32>) {
    let mut rd = hound::WavReader::open(path).unwrap_or_else(|e| die(format!("{path}: {e}")));
    let ch = rd.spec().channels as usize;
    let v: Vec<f32> = match rd.spec().sample_format {
        hound::SampleFormat::Float => rd.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => {
            let s = (1u64 << (rd.spec().bits_per_sample - 1)) as f32;
            rd.samples::<i32>().map(|x| x.unwrap() as f32 / s).collect()
        }
    };
    (v.chunks(ch).map(|f| f[0]).collect(), v.chunks(ch).map(|f| f[1.min(ch - 1)]).collect())
}

/// Null test: `b` against `a`, at the lag (b later, 0..=max_lag) that nulls best.
fn compare(a: &str, b: &str, max_lag: usize) {
    let ((al, ar), (bl, br)) = (read_wav(a), read_wav(b));
    let n = al.len().min(bl.len());
    let resid = |lag: usize, x: &[f32], y: &[f32]| -> Vec<f32> { (0..n.saturating_sub(lag)).map(|i| y[i + lag] - x[i]).collect() };
    let best = (0..=max_lag).min_by(|&p, &q| rms_db(&resid(p, &al, &bl)).total_cmp(&rms_db(&resid(q, &al, &bl)))).unwrap_or(0);
    let (el, er) = (resid(best, &al, &bl), resid(best, &ar, &br));
    let peak = el.iter().chain(&er).fold(0f32, |m, v| m.max(v.abs()));
    println!(
        "lag {best} samples: {a} {:.1} dB RMS, {b} {:.1} dB RMS; residual L {:.1} / R {:.1} dB RMS, peak {:.2e} ({:.1} dBFS)",
        rms_db(&al),
        rms_db(&bl),
        rms_db(&el),
        rms_db(&er),
        peak,
        20.0 * (peak as f64).max(1e-12).log10()
    );
}

/// Give the plug-in a scratch home (unless `MASCHINE_REF_HOME` names one, or is
/// `user`): it otherwise writes its log, preferences, content database (komplete.db3)
/// and user-content folders under the user's home, as in any host.
fn sandbox_home() {
    let home = match std::env::var("MASCHINE_REF_HOME") {
        Ok(v) if v == "user" => return,
        Ok(v) => std::path::PathBuf::from(v),
        Err(_) => std::env::temp_dir().join("maschine-ref-home"),
    };
    let _ = std::fs::create_dir_all(&home);
    // Before any other thread exists (the plug-in isn't loaded yet).
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("CFFIXED_USER_HOME", &home);
    }
}

/// The fake plug-in `plant` adds: a class id no installed plug-in has, and a state.
const PLANT_CID: [u8; 16] = *b"DECKARDtestsynth";

fn plant(args: &[String]) {
    let [src, out, g, s] = args else { die("plant <src.mxprj> <out.mxprj> <g> <s>") };
    let (g, s): (usize, usize) = (g.parse().unwrap_or_else(|_| die("g")), s.parse().unwrap_or_else(|_| die("s")));
    let mut d = Document::open(Path::new(src)).unwrap_or_else(|e| die(e));
    let n = d.project().groups[g].sounds[s].slots.len();
    for i in (0..n).rev() {
        d.remove_slot(g, s, i).unwrap_or_else(|e| die(e));
    }
    let state = State { component: (0..=255u8).cycle().take(1000).collect(), controller: b"ctl".to_vec() };
    let chunk = deckard_plugins::state::wrap_chunk(&state, Format::Vst3, &PLANT_CID);
    let id = format!("DECKARD:VST3:INSTR:{}", PLANT_CID.iter().map(|b| format!("{b:02X}")).collect::<String>());
    let module = deckard_doc::NewModule::Plugin { id, name: "DECKARD Test".into(), chunk };
    d.add_slot(g, s, 0, &module).unwrap_or_else(|e| die(e));
    d.save_as(Path::new(out)).unwrap_or_else(|e| die(e));
    println!("wrote {out}");
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    sandbox_home();
    match args.first().map(String::as_str) {
        Some("info") => info(),
        Some("state") => {
            let out = args.get(1).unwrap_or_else(|| die("state <out.mxprj>"));
            let mut p = maschine(48_000.0);
            let st = p.state();
            std::fs::write(out, &st.component).unwrap_or_else(|e| die(e));
            println!("wrote {out}: component state {} bytes (controller {} bytes)", st.component.len(), st.controller.len());
        }
        Some("list") => list(args.get(1).unwrap_or_else(|| die("list <project.mxprj>"))),
        Some("make") => make(&args[1..]),
        Some("render") => render(&args[1..]),
        Some("roundtrip") => {
            let (src, out) = (args.get(1), args.get(2));
            let (Some(src), Some(out)) = (src, out) else { die("roundtrip <in.mxprj> <out.mxprj>") };
            let bytes = std::fs::read(src).unwrap_or_else(|e| die(format!("{src}: {e}")));
            let mut p = maschine(48_000.0);
            p.set_state(&State { component: bytes, controller: Vec::new() }).unwrap_or_else(|e| die(e));
            let t = Transport { tempo: 120.0, numerator: 4, denominator: 4, ..Default::default() };
            for _ in 0..50 {
                p.process(MAX_BLOCK, &t);
                pump(0.02);
            }
            let st = p.state();
            std::fs::write(out, &st.component).unwrap_or_else(|e| die(e));
            println!("wrote {out}: {} bytes", st.component.len());
        }
        Some("plant") => plant(&args[1..]),
        Some("compare") => {
            let (a, b) = (
                args.get(1).unwrap_or_else(|| die("compare <a.wav> <b.wav> [max-lag]")),
                args.get(2).unwrap_or_else(|| die("compare <a.wav> <b.wav>")),
            );
            compare(a, b, args.get(3).map_or(256, |v| v.parse().unwrap_or_else(|_| die("max-lag"))));
        }
        _ => die("commands: info | state <out> | list <project> | make ... | render ... (see the source header)"),
    }
    deckard_plugins::exit(0);
}
