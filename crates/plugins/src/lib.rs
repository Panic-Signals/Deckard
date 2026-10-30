//! Hosting the plug-ins a Maschine project uses (VST3; VST2 plug-ins through their
//! VST3 versions, see [`state`]).
//!
//! [`Host`] keeps one [`Plugin`] instance per slot of the project (a sound's
//! instrument, say), so they survive the engine being rebuilt after every edit.
//! Instances are created and given their state on the main thread; the audio thread
//! calls [`Plugin::process`] under the instance's mutex.

pub mod com;
pub mod editor;
pub mod module;
pub mod native;
/// Mutable Instruments' licence (Plaits), for the About box.
pub use deckard_mi::LICENSE as MUTABLE_INSTRUMENTS_LICENSE;
mod plaits;
mod replicant;
pub mod scan;
pub mod state;

use com::{ComponentHandler, Edits, EventList, HostApp, ParameterChanges, Stream};
use module::Module;
use state::{PluginId, State};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{ComPtr, ComWrapper, Interface};

unsafe extern "C" {
    fn _exit(code: i32) -> !;
}

/// End the process now. Unlike `std::process::exit`, this skips the exit-time
/// destructors of loaded plug-ins: some (NI's, whose editors use their own Qt) crash
/// in them once the host has started tearing down. Call it instead of returning from
/// `main` in programs that may have loaded plug-ins.
pub fn exit(code: i32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe { _exit(code) }
}

/// Whether this is the process's main thread (where plug-ins must be created).
pub fn on_main_thread() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn pthread_main_np() -> i32;
        }
        unsafe { pthread_main_np() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    true
}

/// Largest block passed to a plug-in; longer requests are split by the caller.
pub const MAX_BLOCK: usize = 1024;

/// The song position, for tempo-synced plug-ins.
#[derive(Debug, Clone, Copy, Default)]
pub struct Transport {
    pub tempo: f64,
    pub playing: bool,
    /// Position in quarter notes.
    pub quarters: f64,
    pub numerator: i32,
    pub denominator: i32,
}

/// Parameters of one plug-in modulation can drive at once.
const MODDED: usize = 64;

/// A running VST3 plug-in.
struct Vst3 {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    /// The controller is a separate object (not the component itself).
    separate: bool,
    /// Processor and controller talk through these when separate.
    connection: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    _host: ComWrapper<HostApp>,
    /// Keeps the factory alive as long as its instances.
    _module: Arc<Module>,
    _handler: ComWrapper<ComponentHandler>,
    edits: Arc<Mutex<Edits>>,
    events: ComWrapper<EventList>,
    params_in: ComWrapper<ParameterChanges>,
    /// Parameters modulation drives (id, value, the knob's own value), room made with
    /// the plug-in.
    modded: Vec<(u32, f64, f64)>,
    params_out: ComWrapper<ParameterChanges>,
    /// [bus][channel] buffers of MAX_BLOCK frames, and pointers to them.
    inputs: Vec<Vec<Vec<f32>>>,
    outputs: Vec<Vec<Vec<f32>>>,
    in_ptrs: Vec<Vec<*mut f32>>,
    out_ptrs: Vec<Vec<*mut f32>>,
    in_buses: Vec<AudioBusBuffers>,
    out_buses: Vec<AudioBusBuffers>,
    sample_rate: f64,
    /// Samples processed so far.
    clock: i64,
    held: [bool; 128],
    /// The parameter each MIDI controller drives (0..=127, then 128 aftertouch, 129
    /// pitch bend), from the plug-in's `IMidiMapping`: VST3 has no MIDI controller
    /// messages, so hosts send them as parameter changes.
    midi_map: [Option<ParamID>; 130],
    /// Controllers moved away from their resting value (to reset on stop).
    moved: [bool; 130],
    active: bool,
    /// The Maschine chunk last applied (to reuse the instance when it hasn't changed).
    chunk: Vec<u8>,
}

// VST3 objects may be used from any thread as long as calls are not concurrent, which
// the mutex around each instance ensures. Processing happens on the audio thread.
unsafe impl Send for Vst3 {}

pub type Shared = Arc<Mutex<Plugin>>;

/// One running plug-in: a VST3, or one of DECKARD's own ([`native`]).
pub struct Plugin {
    pub name: String,
    pub id: PluginId,
    kind: Kind,
}

enum Kind {
    Vst3(Box<Vst3>),
    Native(Box<native::Instance>),
}

/// Call `$e` on whichever plug-in runs.
macro_rules! each {
    ($self:ident, $p:ident => $e:expr) => {
        match &$self.kind {
            Kind::Vst3($p) => $e,
            Kind::Native($p) => $e,
        }
    };
    (mut $self:ident, $p:ident => $e:expr) => {
        match &mut $self.kind {
            Kind::Vst3($p) => $e,
            Kind::Native($p) => $e,
        }
    };
}

impl Plugin {
    /// Create an instance of `id`'s class from `module`, ready to process at
    /// `sample_rate`, with no state set yet.
    pub fn new(module: &Arc<Module>, id: PluginId, name: &str, sample_rate: f64) -> Result<Plugin, String> {
        let v = Vst3::new(module, &id, sample_rate)?;
        Ok(Plugin { name: name.to_string(), id, kind: Kind::Vst3(Box::new(v)) })
    }

    /// An instance of one of DECKARD's own plug-ins, at its default settings.
    pub fn native(class: &'static native::Class, id: PluginId, name: &str, sample_rate: f64) -> Plugin {
        Plugin { name: name.to_string(), id, kind: Kind::Native(Box::new(native::Instance::new(class, sample_rate))) }
    }

    /// Process a few blocks of silence, here rather than on the audio thread: many
    /// plug-ins set themselves up on their first blocks (2–3 ms each for some, most of
    /// a 128-frame block's time). Only for an instance nothing else plays yet.
    pub fn warm_up(&mut self) {
        let silence = [0f32; MAX_BLOCK];
        for _ in 0..4 {
            self.process_input(MAX_BLOCK, &Transport::default(), Some((&silence, &silence)));
        }
    }

    /// Whether this is one of DECKARD's own plug-ins.
    pub fn is_native(&self) -> bool {
        matches!(self.kind, Kind::Native(_))
    }

    pub fn sample_rate(&self) -> f64 {
        each!(self, p => p.sample_rate())
    }

    /// Give the plug-in the state Maschine saved for it (a `PluginHost` chunk), and
    /// start processing.
    pub fn load_chunk(&mut self, chunk: &[u8]) -> Result<(), String> {
        let state = state::unwrap_chunk(chunk, self.id.format)?;
        let r = self.set_state(&state);
        each!(mut self, p => p.chunk = chunk.to_vec());
        r
    }

    /// Set component (and controller) state, and start processing.
    pub fn set_state(&mut self, state: &State) -> Result<(), String> {
        each!(mut self, p => p.set_state(state))
    }

    /// The plug-in's current state.
    pub fn state(&mut self) -> State {
        each!(mut self, p => p.state())
    }

    /// The plug-in's current state, wrapped as Maschine saves it.
    pub fn save_chunk(&mut self) -> Vec<u8> {
        state::wrap_chunk(&self.state(), self.id.format, &self.id.cid)
    }

    /// Record that `chunk` (from [`Plugin::save_chunk`]) is now what the project holds,
    /// so the instance isn't reloaded from it.
    pub fn mark_saved(&mut self, chunk: Vec<u8>) {
        each!(mut self, p => p.chunk = chunk)
    }

    /// The Maschine chunk this instance was loaded from.
    pub fn chunk(&self) -> &[u8] {
        each!(self, p => &p.chunk)
    }

    /// The plug-in's parameters a host may show and change (automatable, not hidden,
    /// read-only, bypass or program change), in the controller's order.
    pub fn parameters(&self) -> Vec<ParamInfo> {
        self.parameter_list(true)
    }

    /// Every parameter of the plug-in, or (`automatable`) those [`Plugin::parameters`]
    /// lists.
    pub fn parameter_list(&self, automatable: bool) -> Vec<ParamInfo> {
        each!(self, p => p.parameter_list(automatable))
    }

    /// The plug-in's own pages of parameters (DECKARD's plug-ins have them): a name,
    /// and up to eight knobs (parameter id, the section heading starting there).
    pub fn pages(&self) -> Vec<(String, Vec<(u32, String)>)> {
        match &self.kind {
            Kind::Native(n) => n.pages(),
            Kind::Vst3(_) => Vec::new(),
        }
    }

    /// A parameter's value (normalized 0..1).
    pub fn param_value(&self, id: u32) -> f64 {
        each!(self, p => p.param_value(id))
    }

    /// How the plug-in shows value `v` of parameter `id`.
    pub fn param_text(&self, id: u32, v: f64) -> String {
        each!(self, p => p.param_text(id, v))
    }

    /// A knob's label as the plug-in's state has it now (DECKARD's Plaits names its
    /// knobs by engine), or `None` for its own.
    pub fn param_label(&self, id: u32) -> Option<String> {
        match &self.kind {
            Kind::Native(n) => n.param_label(id),
            Kind::Vst3(_) => None,
        }
    }

    /// Change a parameter from the host (a knob): the controller shows it and the
    /// processor gets it at the next `process`. Counts as an edit of the state.
    pub fn set_param(&mut self, id: u32, v: f64) {
        each!(mut self, p => p.set_param(id, v))
    }

    /// Something was edited in the plug-in since `take_dirty` was last called.
    pub fn take_dirty(&self) -> bool {
        each!(self, p => p.take_dirty())
    }

    /// Modulation (the audio thread): parameter `id` plays at `v` (normalized) from
    /// the next `process`, leaving the knob (the controller, the saved state) as it is.
    pub fn modulate(&mut self, id: u32, v: f64) {
        each!(mut self, p => p.modulate(id, v))
    }

    /// Modulation of parameter `id` ends: it plays at the knob's value again.
    pub fn unmodulate(&mut self, id: u32) {
        each!(mut self, p => p.unmodulate(id))
    }

    /// The value modulation gives parameter `id`, if it drives it.
    pub fn modulated(&self, id: u32) -> Option<f64> {
        each!(self, p => p.modulated(id))
    }

    /// Start a note at the next `process` (velocity 1..127).
    pub fn note_on(&mut self, pitch: i64, velocity: i64) {
        each!(mut self, p => p.note_on(pitch, velocity))
    }

    /// End a note at the next `process`.
    pub fn note_off(&mut self, pitch: i64) {
        each!(mut self, p => p.note_off(pitch))
    }

    /// End every sounding note, and let go of the sustain pedal and pitch bend.
    pub fn all_notes_off(&mut self) {
        each!(mut self, p => p.all_notes_off())
    }

    /// Whether the plug-in takes MIDI controller `number` (0..=127; 129 = pitch bend).
    pub fn takes_controller(&self, number: usize) -> bool {
        each!(self, p => p.takes_controller(number))
    }

    /// A MIDI control change (value 0..127), at the next `process`.
    pub fn control_change(&mut self, number: u8, value: u8) {
        each!(mut self, p => p.control_change(number, value))
    }

    /// Pitch bend (0..16383, 8192 = centre), at the next `process`.
    pub fn pitch_bend(&mut self, value: u16) {
        each!(mut self, p => p.pitch_bend(value))
    }

    /// Run the plug-in for `frames` frames (at most [`MAX_BLOCK`]); returns its main
    /// output (left, right).
    pub fn process(&mut self, frames: usize, transport: &Transport) -> (&[f32], &[f32]) {
        self.process_input(frames, transport, None)
    }

    /// [`Plugin::process`] with `input` (left, right) on the plug-in's main input: an
    /// effect.
    pub fn process_input(&mut self, frames: usize, transport: &Transport, input: Option<(&[f32], &[f32])>) -> (&[f32], &[f32]) {
        each!(mut self, p => p.process_input(frames, transport, input))
    }

    /// A way to read and change a VST3 plug-in's parameters without holding the
    /// instance (so the audio thread never waits on the UI): VST3 controllers run on
    /// the UI thread alongside processing. Use it on the thread that made the plug-in.
    /// None for DECKARD's own plug-ins (reading those under the lock is instant).
    pub fn params(&self) -> Option<Params> {
        match &self.kind {
            Kind::Vst3(p) => Some(Params { controller: p.controller.clone()?, edits: p.edits.clone() }),
            Kind::Native(_) => None,
        }
    }

    /// The VST3 edit controller (none for DECKARD's own plug-ins).
    pub fn controller(&self) -> Option<&ComPtr<IEditController>> {
        match &self.kind {
            Kind::Vst3(p) => p.controller.as_ref(),
            Kind::Native(_) => None,
        }
    }

    /// Output bus `bus` (left, right) of the last `process` call of `frames` frames.
    pub fn output_bus(&self, bus: usize, frames: usize) -> Option<(&[f32], &[f32])> {
        each!(self, p => p.output_bus(bus, frames))
    }

    /// The VST3 component (to query buses and the like); none for DECKARD's own.
    pub fn component(&self) -> Option<&ComPtr<IComponent>> {
        match &self.kind {
            Kind::Vst3(p) => Some(&p.component),
            Kind::Native(_) => None,
        }
    }
}

fn ok(r: tresult, what: &str) -> Result<(), String> {
    if r == kResultOk { Ok(()) } else { Err(format!("{what} failed ({r})")) }
}

impl Vst3 {
    fn new(module: &Arc<Module>, id: &PluginId, sample_rate: f64) -> Result<Vst3, String> {
        let host = ComWrapper::new(HostApp);
        let host_ptr = host.as_com_ref::<IHostApplication>().unwrap().as_ptr() as *mut FUnknown;
        let mut obj = std::ptr::null_mut();
        let cid = id.cid;
        unsafe {
            ok(
                module.factory.createInstance(cid.as_ptr() as FIDString, IComponent::IID.as_ptr() as FIDString, &mut obj),
                "creating the plug-in",
            )?;
        }
        let component = unsafe { ComPtr::from_raw(obj as *mut IComponent) }.ok_or("the plug-in has no component")?;
        unsafe { ok(component.initialize(host_ptr), "initializing the plug-in")? };
        let processor = component.cast::<IAudioProcessor>().ok_or("the plug-in has no audio processor")?;

        // The edit controller: the component itself, or a separate class.
        let mut controller = component.cast::<IEditController>();
        let mut separate = false;
        if controller.is_none() {
            let mut ccid: TUID = [0; 16];
            if unsafe { component.getControllerClassId(&mut ccid) } == kResultOk {
                let mut obj = std::ptr::null_mut();
                let r = unsafe {
                    module.factory.createInstance(ccid.as_ptr() as FIDString, IEditController::IID.as_ptr() as FIDString, &mut obj)
                };
                if r == kResultOk
                    && let Some(c) = unsafe { ComPtr::from_raw(obj as *mut IEditController) }
                    && unsafe { c.initialize(host_ptr) } == kResultOk
                {
                    controller = Some(c);
                    separate = true;
                }
            }
        }
        let mut connection = None;
        if separate
            && let Some(c) = &controller
            && let (Some(a), Some(b)) = (component.cast::<IConnectionPoint>(), c.cast::<IConnectionPoint>())
        {
            unsafe {
                a.connect(b.as_ptr());
                b.connect(a.as_ptr());
            }
            connection = Some((a, b));
        }
        let edits = Arc::new(Mutex::new(Edits::default()));
        let handler = ComWrapper::new(ComponentHandler { edits: edits.clone() });
        if let Some(c) = &controller {
            unsafe { c.setComponentHandler(handler.as_com_ref::<IComponentHandler>().unwrap().as_ptr()) };
        }

        // Buses: every audio bus on, and the first event input.
        let (audio, event) = (MediaTypes_::kAudio as MediaType, MediaTypes_::kEvent as MediaType);
        let (input, output) = (BusDirections_::kInput as BusDirection, BusDirections_::kOutput as BusDirection);
        let buses = |dir| {
            (0..unsafe { component.getBusCount(audio, dir) })
                .map(|i| {
                    let mut info: BusInfo = unsafe { std::mem::zeroed() };
                    unsafe {
                        component.getBusInfo(audio, dir, i, &mut info);
                        component.activateBus(audio, dir, i, 1);
                    }
                    vec![vec![0f32; MAX_BLOCK]; info.channelCount.max(0) as usize]
                })
                .collect::<Vec<_>>()
        };
        let inputs = buses(input);
        let outputs = buses(output);
        if unsafe { component.getBusCount(event, input) } > 0 {
            unsafe { component.activateBus(event, input, 0, 1) };
        }
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            maxSamplesPerBlock: MAX_BLOCK as i32,
            sampleRate: sample_rate,
        };
        unsafe { ok(processor.setupProcessing(&mut setup), "setting up processing")? };

        let mut midi_map = [None; 130];
        if let Some(m) = controller.as_ref().and_then(|c| c.cast::<IMidiMapping>()) {
            for (cc, slot) in midi_map.iter_mut().enumerate() {
                let mut id: ParamID = 0;
                if unsafe { m.getMidiControllerAssignment(0, 0, cc as CtrlNumber, &mut id) } == kResultOk {
                    *slot = Some(id);
                }
            }
        }
        let mut p = Vst3 {
            component,
            processor,
            controller,
            separate,
            connection,
            _host: host,
            _module: module.clone(),
            _handler: handler,
            edits,
            events: ComWrapper::new(EventList::default()),
            params_in: ComWrapper::new(ParameterChanges::default()),
            modded: Vec::with_capacity(MODDED),
            params_out: ComWrapper::new(ParameterChanges::default()),
            inputs,
            outputs,
            in_ptrs: Vec::new(),
            out_ptrs: Vec::new(),
            in_buses: Vec::new(),
            out_buses: Vec::new(),
            sample_rate,
            clock: 0,
            held: [false; 128],
            midi_map,
            moved: [false; 130],
            active: false,
            chunk: Vec::new(),
        };
        // The buffers never move after this, so the pointers stay valid.
        p.in_ptrs = p.inputs.iter_mut().map(|b| b.iter_mut().map(|c| c.as_mut_ptr()).collect()).collect();
        p.out_ptrs = p.outputs.iter_mut().map(|b| b.iter_mut().map(|c| c.as_mut_ptr()).collect()).collect();
        let bus = |p: &mut Vec<*mut f32>| AudioBusBuffers {
            numChannels: p.len() as i32,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 { channelBuffers32: p.as_mut_ptr() },
        };
        p.in_buses = p.in_ptrs.iter_mut().map(bus).collect();
        p.out_buses = p.out_ptrs.iter_mut().map(bus).collect();
        Ok(p)
    }

    fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    fn set_active(&mut self, on: bool) {
        if self.active == on {
            return;
        }
        unsafe {
            if on {
                self.component.setActive(1);
                self.processor.setProcessing(1);
            } else {
                self.processor.setProcessing(0);
                self.component.setActive(0);
            }
        }
        self.active = on;
    }

    fn set_state(&mut self, state: &State) -> Result<(), String> {
        self.set_active(false);
        let mut result = Ok(());
        if !state.component.is_empty() {
            let s = Stream::new(state.component.clone());
            let r = unsafe { self.component.setState(s.as_com_ref::<IBStream>().unwrap().as_ptr()) };
            result = ok(r, "restoring the plug-in's state");
            if let Some(c) = &self.controller {
                let s = Stream::new(state.component.clone());
                unsafe { c.setComponentState(s.as_com_ref::<IBStream>().unwrap().as_ptr()) };
            }
        }
        if !state.controller.is_empty()
            && let Some(c) = &self.controller
        {
            let s = Stream::new(state.controller.clone());
            unsafe { c.setState(s.as_com_ref::<IBStream>().unwrap().as_ptr()) };
        }
        self.held = [false; 128];
        self.set_active(true);
        result
    }

    /// The state, with each parameter modulation drives at its knob's value: the
    /// processor gets those for the moment it's asked (BUGS #16).
    fn state(&mut self) -> State {
        let modded = self.modded.clone();
        if !modded.is_empty() {
            for &(id, _, knob) in &modded {
                self.params_in.add(id, knob);
            }
            self.flush_params();
        }
        let state = self.read_state();
        if !modded.is_empty() {
            for &(id, v, _) in &modded {
                self.params_in.add(id, v);
            }
            self.flush_params();
        }
        state
    }

    /// Hand the processor the parameter changes waiting, at once: a process call of no
    /// frames, as VST3 allows for this (no notes go with it).
    fn flush_params(&mut self) {
        if !self.active || self.outputs.first().is_none_or(|b| b.is_empty()) {
            return;
        }
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: 0,
            numInputs: self.in_buses.len() as i32,
            numOutputs: self.out_buses.len() as i32,
            inputs: if self.in_buses.is_empty() { std::ptr::null_mut() } else { self.in_buses.as_mut_ptr() },
            outputs: self.out_buses.as_mut_ptr(),
            inputParameterChanges: self.params_in.as_com_ref::<IParameterChanges>().unwrap().as_ptr(),
            outputParameterChanges: self.params_out.as_com_ref::<IParameterChanges>().unwrap().as_ptr(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: std::ptr::null_mut(),
        };
        unsafe { self.processor.process(&mut data) };
        self.params_in.clear();
        self.params_out.clear();
    }

    fn read_state(&self) -> State {
        let s = Stream::new(Vec::new());
        unsafe { self.component.getState(s.as_com_ref::<IBStream>().unwrap().as_ptr()) };
        let component = s.take();
        let controller = match &self.controller {
            Some(c) if self.separate => {
                let s = Stream::new(Vec::new());
                unsafe { c.getState(s.as_com_ref::<IBStream>().unwrap().as_ptr()) };
                s.take()
            }
            _ => Vec::new(),
        };
        State { component, controller }
    }

    /// Every parameter of the plug-in, or (`automatable`) those [`Plugin::parameters`]
    /// lists.
    pub fn parameter_list(&self, automatable: bool) -> Vec<ParamInfo> {
        let Some(c) = &self.controller else { return Vec::new() };
        let n = unsafe { c.getParameterCount() };
        let mut out = Vec::new();
        for i in 0..n {
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            if unsafe { c.getParameterInfo(i, &mut info) } != kResultOk {
                continue;
            }
            use ParameterInfo_::ParameterFlags_::*;
            let f = info.flags;
            if automatable && (f & kCanAutomate == 0 || f & (kIsHidden | kIsReadOnly | kIsBypass | kIsProgramChange) != 0) {
                continue;
            }
            out.push(ParamInfo {
                id: info.id,
                title: string128(&info.title),
                short: string128(&info.shortTitle),
                units: string128(&info.units),
                steps: info.stepCount,
                default: info.defaultNormalizedValue,
            });
        }
        out
    }

    /// A parameter's value (normalized 0..1).
    pub fn param_value(&self, id: u32) -> f64 {
        self.controller.as_ref().map_or(0.0, |c| unsafe { c.getParamNormalized(id) })
    }

    /// How the plug-in shows value `v` of parameter `id`.
    pub fn param_text(&self, id: u32, v: f64) -> String {
        let Some(c) = &self.controller else { return String::new() };
        let mut s: String128 = [0; 128];
        if unsafe { c.getParamStringByValue(id, v, &mut s) } == kResultOk { string128(&s) } else { format!("{:.0} %", v * 100.0) }
    }

    /// Change a parameter from the host (a knob): the controller shows it and the
    /// processor gets it at the next `process`. Counts as an edit of the state.
    pub fn set_param(&mut self, id: u32, v: f64) {
        let v = v.clamp(0.0, 1.0);
        if let Some(c) = &self.controller {
            unsafe { c.setParamNormalized(id, v) };
        }
        if let Ok(mut e) = self.edits.lock() {
            e.pending.push((id, v));
            e.dirty = true;
        }
    }

    /// Something was edited in the plug-in since `take_dirty` was last called.
    pub fn take_dirty(&self) -> bool {
        self.edits.lock().map(|mut e| std::mem::take(&mut e.dirty)).unwrap_or(false)
    }

    fn event(&self, r#type: Event_::EventTypes, data: Event__type0) {
        let e = Event {
            busIndex: 0,
            sampleOffset: 0,
            ppqPosition: 0.0,
            flags: Event_::EventFlags_::kIsLive as u16,
            r#type: r#type as u16,
            __field0: data,
        };
        self.events.events.borrow_mut().push(e);
    }

    /// Start a note at the next `process` (velocity 1..127).
    pub fn note_on(&mut self, pitch: i64, velocity: i64) {
        let pitch = pitch.clamp(0, 127);
        if self.held[pitch as usize] {
            self.note_off(pitch);
        }
        self.held[pitch as usize] = true;
        let on = NoteOnEvent {
            channel: 0,
            pitch: pitch as i16,
            tuning: 0.0,
            velocity: velocity.clamp(1, 127) as f32 / 127.0,
            length: 0,
            noteId: -1,
        };
        self.event(Event_::EventTypes_::kNoteOnEvent, Event__type0 { noteOn: on });
    }

    /// End a note at the next `process`.
    pub fn note_off(&mut self, pitch: i64) {
        let pitch = pitch.clamp(0, 127);
        if !std::mem::take(&mut self.held[pitch as usize]) {
            return;
        }
        let off = NoteOffEvent { channel: 0, pitch: pitch as i16, velocity: 0.0, noteId: -1, tuning: 0.0 };
        self.event(Event_::EventTypes_::kNoteOffEvent, Event__type0 { noteOff: off });
    }

    /// End every sounding note, and let go of the sustain pedal and pitch bend.
    pub fn all_notes_off(&mut self) {
        for p in 0..128 {
            self.note_off(p);
        }
        if self.moved[64] {
            self.control_change(64, 0);
        }
        if self.moved[129] {
            self.pitch_bend(8192);
        }
    }

    /// Whether the plug-in takes MIDI controller `number` (0..=127; 129 = pitch bend).
    pub fn takes_controller(&self, number: usize) -> bool {
        self.midi_map.get(number).is_some_and(Option::is_some)
    }

    fn modulate(&mut self, id: u32, v: f64) {
        let v = v.clamp(0.0, 1.0);
        self.params_in.add(id, v);
        if let Some(m) = self.modded.iter_mut().find(|m| m.0 == id) {
            m.1 = v;
        } else if self.modded.len() < MODDED {
            // The knob's value, kept: a plug-in whose processor and controller are one
            // shows the modulation as its value.
            let knob = self.param_value(id);
            self.modded.push((id, v, knob));
        }
    }

    fn unmodulate(&mut self, id: u32) {
        let Some(i) = self.modded.iter().position(|m| m.0 == id) else { return };
        let (_, _, knob) = self.modded.remove(i);
        self.params_in.add(id, knob);
        if let Some(c) = &self.controller {
            unsafe { c.setParamNormalized(id, knob) };
        }
    }

    fn modulated(&self, id: u32) -> Option<f64> {
        self.modded.iter().find(|m| m.0 == id).map(|m| m.1)
    }

    /// A MIDI control change (value 0..127), at the next `process`.
    pub fn control_change(&mut self, number: u8, value: u8) {
        let n = number.min(127) as usize;
        if let Some(id) = self.midi_map[n] {
            self.params_in.add(id, value.min(127) as f64 / 127.0);
            self.moved[n] = value != 0;
        }
    }

    /// Pitch bend (0..16383, 8192 = centre), at the next `process`.
    pub fn pitch_bend(&mut self, value: u16) {
        if let Some(id) = self.midi_map[129] {
            self.params_in.add(id, value.min(16383) as f64 / 16383.0);
            self.moved[129] = value != 8192;
        }
    }

    /// [`Plugin::process`] with `input` (left, right) on the plug-in's main input: an
    /// effect.
    pub fn process_input(&mut self, frames: usize, transport: &Transport, input: Option<(&[f32], &[f32])>) -> (&[f32], &[f32]) {
        let frames = frames.min(MAX_BLOCK);
        if !self.active || frames == 0 || self.outputs.first().is_none_or(|b| b.is_empty()) {
            self.events.events.borrow_mut().clear();
            return (&[], &[]);
        }
        // Edits from the plug-in's editor go to the processor too.
        // (Not those modulation drives: it holds them until it lets go.)
        if let Ok(mut e) = self.edits.try_lock() {
            for (id, v) in e.pending.drain(..) {
                match self.modded.iter_mut().find(|m| m.0 == id) {
                    Some(m) => m.2 = v,
                    None => self.params_in.add(id, v),
                }
            }
        }
        for ch in self.inputs.iter_mut().flatten() {
            ch[..frames].fill(0.0);
        }
        for b in &mut self.in_buses {
            b.silenceFlags = u64::MAX;
        }
        if let Some((l, r)) = input
            && let Some(main) = self.inputs.first_mut()
        {
            let n = frames.min(l.len()).min(r.len());
            match main.len() {
                0 => {}
                1 => {
                    for (o, (a, b)) in main[0][..n].iter_mut().zip(l.iter().zip(r)) {
                        *o = (a + b) * 0.5;
                    }
                }
                _ => {
                    main[0][..n].copy_from_slice(&l[..n]);
                    main[1][..n].copy_from_slice(&r[..n]);
                }
            }
            if let Some(b) = self.in_buses.first_mut() {
                b.silenceFlags = 0;
            }
        }
        for b in &mut self.out_buses {
            b.silenceFlags = 0;
        }
        let mut ctx: ProcessContext = unsafe { std::mem::zeroed() };
        {
            use ProcessContext_::StatesAndFlags_::*;
            // C enum constants: u32 on macOS, i32 on Windows.
            #[allow(clippy::unnecessary_cast)]
            let flags =
                (kTempoValid | kTimeSigValid | kProjectTimeMusicValid | kContTimeValid | if transport.playing { kPlaying } else { 0 })
                    as u32;
            ctx.state = flags;
        }
        ctx.sampleRate = self.sample_rate;
        ctx.projectTimeSamples = self.clock;
        ctx.continousTimeSamples = self.clock;
        ctx.projectTimeMusic = transport.quarters;
        ctx.tempo = if transport.tempo > 0.0 { transport.tempo } else { 120.0 };
        ctx.timeSigNumerator = transport.numerator.max(1);
        ctx.timeSigDenominator = transport.denominator.max(1);
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: frames as i32,
            numInputs: self.in_buses.len() as i32,
            numOutputs: self.out_buses.len() as i32,
            inputs: if self.in_buses.is_empty() { std::ptr::null_mut() } else { self.in_buses.as_mut_ptr() },
            outputs: self.out_buses.as_mut_ptr(),
            inputParameterChanges: self.params_in.as_com_ref::<IParameterChanges>().unwrap().as_ptr(),
            outputParameterChanges: self.params_out.as_com_ref::<IParameterChanges>().unwrap().as_ptr(),
            inputEvents: self.events.as_com_ref::<IEventList>().unwrap().as_ptr(),
            outputEvents: std::ptr::null_mut(),
            processContext: &mut ctx,
        };
        unsafe { self.processor.process(&mut data) };
        self.events.events.borrow_mut().clear();
        self.params_in.clear();
        // Parameter changes the processor reports go to the controller (its editor).
        if self.separate
            && let Some(c) = &self.controller
        {
            for (id, v) in self.params_out.last_values() {
                unsafe { c.setParamNormalized(id, v) };
            }
        }
        self.params_out.clear();
        self.clock += frames as i64;
        let main = &self.outputs[0];
        let left = &main[0][..frames];
        let right = &main[main.len().min(2) - 1][..frames];
        (left, right)
    }

    /// Output bus `bus` (left, right) of the last `process` call of `frames` frames.
    pub fn output_bus(&self, bus: usize, frames: usize) -> Option<(&[f32], &[f32])> {
        let b = self.outputs.get(bus).filter(|b| !b.is_empty())?;
        let n = frames.min(MAX_BLOCK);
        Some((&b[0][..n], &b[b.len().min(2) - 1][..n]))
    }
}

impl Drop for Vst3 {
    fn drop(&mut self) {
        self.set_active(false);
        unsafe {
            if let Some((a, b)) = self.connection.take() {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(c) = &self.controller {
                c.setComponentHandler(std::ptr::null_mut());
                if self.separate {
                    c.terminate();
                }
            }
            self.component.terminate();
        }
    }
}

/// A VST3 plug-in's parameters, reached through its controller (see
/// [`Plugin::params`]).
pub struct Params {
    controller: ComPtr<IEditController>,
    edits: Arc<Mutex<Edits>>,
}

impl Params {
    /// A parameter's value (normalized 0..1).
    pub fn value(&self, id: u32) -> f64 {
        unsafe { self.controller.getParamNormalized(id) }
    }

    /// How the plug-in shows value `v` of parameter `id`.
    pub fn text(&self, id: u32, v: f64) -> String {
        let mut s: String128 = [0; 128];
        if unsafe { self.controller.getParamStringByValue(id, v, &mut s) } == kResultOk {
            string128(&s)
        } else {
            format!("{:.0} %", v * 100.0)
        }
    }

    /// Change a parameter (as [`Plugin::set_param`]): the processor gets it at its
    /// next block.
    pub fn set(&self, id: u32, v: f64) {
        let v = v.clamp(0.0, 1.0);
        unsafe { self.controller.setParamNormalized(id, v) };
        if let Ok(mut e) = self.edits.lock() {
            e.pending.push((id, v));
            e.dirty = true;
        }
    }
}

/// A plug-in parameter, as [`Plugin::parameters`] lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    pub id: u32,
    pub title: String,
    pub short: String,
    pub units: String,
    /// 0 = continuous, else the number of steps.
    pub steps: i32,
    pub default: f64,
}

fn string128(s: &[u16]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

/// Which plug-in slot of the project an instance plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// A sound's instrument.
    Sound { group: usize, sound: usize },
    /// Another slot of a sound's chain (an effect), by position.
    Chain { group: usize, sound: usize, slot: usize },
    /// A slot of a group's chain.
    Group { group: usize, slot: usize },
    /// A slot of the master's chain.
    Master { slot: usize },
}

/// The plug-in instances of an open project, and the modules they come from.
#[derive(Default)]
pub struct Host {
    modules: Vec<Arc<Module>>,
    /// Bundles that failed to load (path → why).
    broken: HashMap<std::path::PathBuf, String>,
    instances: HashMap<Slot, Shared>,
}

impl Host {
    /// The instance `slot` has now, if any (none is made).
    pub fn existing(&self, slot: Slot) -> Option<Shared> {
        self.instances.get(&slot).cloned()
    }

    /// The instance for `slot`, playing plug-in `plugin_id` (Maschine's id) named
    /// `name` with Maschine's saved `chunk`. Reuses the slot's instance when it is the
    /// same plug-in, and loads the chunk again only if it changed.
    pub fn instance(&mut self, slot: Slot, plugin_id: &str, name: &str, chunk: &[u8], sample_rate: f64) -> Result<Shared, String> {
        let id = PluginId::parse(plugin_id, name).ok_or_else(|| format!("{name}: plug-in format not supported ({plugin_id})"))?;
        // DECKARD's own plug-ins need no module, and may be made on any thread.
        if let Some(class) = native::find(&id.cid) {
            if let Some(shared) = self.instances.get(&slot) {
                let mut p = shared.lock().unwrap();
                if p.id.cid == id.cid && p.sample_rate() == sample_rate {
                    if p.chunk() != chunk {
                        p.load_chunk(chunk).map_err(|e| format!("{name}: {e}"))?;
                    }
                    drop(p);
                    return Ok(shared.clone());
                }
            }
            let mut p = Plugin::native(class, id, name, sample_rate);
            p.load_chunk(chunk).map_err(|e| format!("{name}: {e}"))?;
            p.warm_up();
            let shared = Arc::new(Mutex::new(p));
            self.instances.insert(slot, shared.clone());
            return Ok(shared);
        }
        if !on_main_thread() {
            // Plug-ins with their own UI toolkit (NI's use Qt) abort if created elsewhere.
            return Err(format!("{name}: plug-ins can only be loaded on the main thread"));
        }
        if let Some(shared) = self.instances.get(&slot) {
            let mut p = shared.lock().unwrap();
            if p.id.cid == id.cid && p.sample_rate() == sample_rate {
                if p.chunk() != chunk {
                    p.load_chunk(chunk).map_err(|e| format!("{name}: {e}"))?;
                }
                drop(p);
                return Ok(shared.clone());
            }
        }
        let module = self.module_for(&id.cid, name).ok_or_else(|| format!("{name}: not installed as VST3"))?;
        let mut p = Plugin::new(&module, id, name, sample_rate).map_err(|e| format!("{name}: {e}"))?;
        p.load_chunk(chunk).map_err(|e| format!("{name}: {e}"))?;
        p.warm_up();
        let shared = Arc::new(Mutex::new(p));
        self.instances.insert(slot, shared.clone());
        Ok(shared)
    }

    /// The instance already playing `slot` as plug-in `plugin_id`, if one is loaded
    /// (nothing is loaded here).
    pub fn loaded(&self, slot: Slot, plugin_id: &str, name: &str) -> Option<Shared> {
        let id = PluginId::parse(plugin_id, name)?;
        let shared = self.instances.get(&slot)?;
        let same = shared.lock().ok()?.id.cid == id.cid;
        same.then(|| shared.clone())
    }

    /// Drop instances whose slot `keep` rejects (e.g. sounds that no longer exist).
    pub fn retain(&mut self, keep: impl Fn(&Slot) -> bool) {
        self.instances.retain(|s, _| keep(s));
    }

    pub fn instances(&self) -> impl Iterator<Item = (&Slot, &Shared)> {
        self.instances.iter()
    }

    /// Bundles that could not be loaded, and why.
    pub fn broken(&self) -> impl Iterator<Item = (&std::path::Path, &str)> {
        self.broken.iter().map(|(p, e)| (p.as_path(), e.as_str()))
    }

    /// The loaded module that has class `cid`, loading it if needed: first bundles that
    /// declare it, then ones named like the plug-in, then any others that don't
    /// declare their classes.
    fn module_for(&mut self, cid: &[u8; 16], name: &str) -> Option<Arc<Module>> {
        if let Some(m) = self.modules.iter().find(|m| m.has(cid)) {
            return Some(m.clone());
        }
        let loaded: Vec<_> = self.modules.iter().map(|m| m.path.clone()).collect();
        let lname = name.to_lowercase();
        let mut candidates: Vec<(u8, std::path::PathBuf)> = module::bundles()
            .into_iter()
            .filter(|b| !loaded.contains(b) && !self.broken.contains_key(b))
            .filter_map(|b| {
                let declared = module::declared_classes(&b);
                let rank = match declared {
                    Some(d) if d.contains(cid) => 0,
                    Some(_) => return None,
                    None if b.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == lname) => 1,
                    None => 2,
                };
                Some((rank, b))
            })
            .collect();
        candidates.sort();
        for (_, b) in candidates {
            match Module::load(&b) {
                Ok(m) => {
                    let m = Arc::new(m);
                    self.modules.push(m.clone());
                    if m.has(cid) {
                        return Some(m);
                    }
                }
                Err(e) => {
                    self.broken.insert(b, e);
                }
            }
        }
        None
    }
}
