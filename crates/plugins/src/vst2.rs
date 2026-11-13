//! Hosting VST 2.4 plug-ins directly: a `.so` (Linux), `.dll` (Windows) or `.vst`
//! bundle (macOS), or an MPC plug-in folder (`plugin.json` naming a binary per CPU).
//!
//! The binary interface is written here from its public description (the layout of the
//! plug-in's `AEffect`, the opcodes, the event and time structures), as open-source hosts
//! and inMusic's MIT-licensed MPC plug-in SDK do; no Steinberg SDK code is used.
//!
//! What a project saves for a VST2 plug-in is what the plug-in returns for its state
//! (`effGetChunk`, bank form) if it keeps its state as a chunk, else its parameters as a
//! standard `.fxp` program (`CcnK`/`FxCk`).

use crate::{MAX_BLOCK, MODDED, ParamInfo, State, Transport};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// The binary interface: the structures and numbers a VST 2.4 plug-in and its host share.
pub(crate) mod abi {
    use std::ffi::c_void;

    /// `'VstP'`, the first field of every `AEffect`.
    pub const MAGIC: i32 = i32::from_be_bytes(*b"VstP");

    pub type HostCallback = unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;
    pub type Dispatcher = unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;
    pub type Process = unsafe extern "C" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32);
    pub type SetParameter = unsafe extern "C" fn(*mut AEffect, i32, f32);
    pub type GetParameter = unsafe extern "C" fn(*mut AEffect, i32) -> f32;
    /// `VSTPluginMain` (and the older `main` / `main_macho`).
    pub type Entry = unsafe extern "C" fn(HostCallback) -> *mut AEffect;

    /// The plug-in, as its entry point returns it.
    #[repr(C)]
    pub struct AEffect {
        pub magic: i32,
        pub dispatcher: Option<Dispatcher>,
        /// The old accumulating process (deprecated; used only if there's no other).
        pub process: Option<Process>,
        pub set_parameter: Option<SetParameter>,
        pub get_parameter: Option<GetParameter>,
        pub num_programs: i32,
        pub num_params: i32,
        pub num_inputs: i32,
        pub num_outputs: i32,
        pub flags: i32,
        pub resvd1: isize,
        pub resvd2: isize,
        pub initial_delay: i32,
        pub real_qualities: i32,
        pub off_qualities: i32,
        pub io_ratio: f32,
        /// The plug-in's own object.
        pub object: *mut c_void,
        /// The host's: DECKARD keeps its side of the instance here.
        pub user: *mut c_void,
        pub unique_id: i32,
        pub version: i32,
        pub process_replacing: Option<Process>,
        pub process_double_replacing: Option<unsafe extern "C" fn(*mut AEffect, *mut *mut f64, *mut *mut f64, i32)>,
        pub future: [u8; 56],
    }

    // Plug-in flags.
    pub const FLAG_HAS_EDITOR: i32 = 1;
    pub const FLAG_CAN_REPLACING: i32 = 1 << 4;
    pub const FLAG_PROGRAM_CHUNKS: i32 = 1 << 5;
    pub const FLAG_IS_SYNTH: i32 = 1 << 8;

    // What the host asks the plug-in (`dispatcher` opcodes).
    pub const EFF_OPEN: i32 = 0;
    pub const EFF_CLOSE: i32 = 1;
    pub const EFF_GET_PARAM_LABEL: i32 = 6;
    pub const EFF_GET_PARAM_DISPLAY: i32 = 7;
    pub const EFF_GET_PARAM_NAME: i32 = 8;
    pub const EFF_SET_SAMPLE_RATE: i32 = 10;
    pub const EFF_SET_BLOCK_SIZE: i32 = 11;
    pub const EFF_MAINS_CHANGED: i32 = 12;
    pub const EFF_GET_CHUNK: i32 = 23;
    pub const EFF_SET_CHUNK: i32 = 24;
    pub const EFF_PROCESS_EVENTS: i32 = 25;
    pub const EFF_CAN_BE_AUTOMATED: i32 = 26;
    pub const EFF_GET_OUTPUT_PROPERTIES: i32 = 34;
    pub const EFF_GET_PLUG_CATEGORY: i32 = 35;
    pub const EFF_GET_EFFECT_NAME: i32 = 45;
    pub const EFF_GET_VENDOR_STRING: i32 = 47;
    pub const EFF_GET_PRODUCT_STRING: i32 = 48;
    pub const EFF_CAN_DO: i32 = 51;
    pub const EFF_START_PROCESS: i32 = 71;
    pub const EFF_STOP_PROCESS: i32 = 72;
    pub const EFF_SET_PROCESS_PRECISION: i32 = 77;

    /// `effGetPlugCategory`: an instrument, and a shell (many plug-ins in one library,
    /// not supported).
    pub const CATEGORY_SYNTH: isize = 2;
    pub const CATEGORY_SHELL: isize = 10;

    // What the plug-in asks the host (`audioMaster` opcodes).
    pub const HOST_AUTOMATE: i32 = 0;
    pub const HOST_VERSION: i32 = 1;
    pub const HOST_CURRENT_ID: i32 = 2;
    pub const HOST_IDLE: i32 = 3;
    pub const HOST_WANT_MIDI: i32 = 6;
    pub const HOST_GET_TIME: i32 = 7;
    pub const HOST_IO_CHANGED: i32 = 13;
    pub const HOST_GET_SAMPLE_RATE: i32 = 16;
    pub const HOST_GET_BLOCK_SIZE: i32 = 17;
    pub const HOST_GET_CURRENT_PROCESS_LEVEL: i32 = 23;
    pub const HOST_GET_AUTOMATION_STATE: i32 = 24;
    pub const HOST_GET_VENDOR_STRING: i32 = 32;
    pub const HOST_GET_PRODUCT_STRING: i32 = 33;
    pub const HOST_GET_VENDOR_VERSION: i32 = 34;
    pub const HOST_CAN_DO: i32 = 37;
    pub const HOST_GET_LANGUAGE: i32 = 38;
    pub const HOST_UPDATE_DISPLAY: i32 = 42;
    pub const HOST_BEGIN_EDIT: i32 = 43;
    pub const HOST_END_EDIT: i32 = 44;

    /// The song position, as `audioMasterGetTime` returns it.
    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub struct TimeInfo {
        pub sample_pos: f64,
        pub sample_rate: f64,
        pub nano_seconds: f64,
        pub ppq_pos: f64,
        pub tempo: f64,
        pub bar_start_pos: f64,
        pub cycle_start_pos: f64,
        pub cycle_end_pos: f64,
        pub time_sig_numerator: i32,
        pub time_sig_denominator: i32,
        pub smpte_offset: i32,
        pub smpte_frame_rate: i32,
        pub samples_to_next_clock: i32,
        pub flags: i32,
    }

    pub const TIME_TRANSPORT_CHANGED: i32 = 1;
    pub const TIME_PLAYING: i32 = 1 << 1;
    pub const TIME_PPQ_VALID: i32 = 1 << 9;
    pub const TIME_TEMPO_VALID: i32 = 1 << 10;
    pub const TIME_BARS_VALID: i32 = 1 << 11;
    pub const TIME_SIG_VALID: i32 = 1 << 13;

    /// One MIDI message for `effProcessEvents` (32 bytes, as every `VstEvent` is).
    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub struct MidiEvent {
        /// 1: MIDI.
        pub kind: i32,
        pub byte_size: i32,
        pub delta_frames: i32,
        pub flags: i32,
        pub note_length: i32,
        pub note_offset: i32,
        pub midi_data: [u8; 4],
        pub detune: i8,
        pub note_off_velocity: u8,
        pub reserved1: u8,
        pub reserved2: u8,
    }

    pub const MIDI_TYPE: i32 = 1;
    /// Played live (not from a sequence): the plug-in should give it priority.
    pub const MIDI_REALTIME: i32 = 1;

    /// Most events sent before one block.
    pub const EVENTS: usize = 256;

    /// `VstEvents`: a count and pointers to the events (the interface declares two and
    /// lets hosts allocate more).
    #[repr(C)]
    pub struct Events {
        pub num_events: i32,
        pub reserved: isize,
        pub events: [*mut MidiEvent; EVENTS],
    }

    /// `effGetOutputProperties`.
    #[repr(C)]
    pub struct PinProperties {
        pub label: [u8; 64],
        pub flags: i32,
        pub arrangement_type: i32,
        pub short_label: [u8; 8],
        pub future: [u8; 48],
    }
}

use abi::AEffect;

/// DECKARD's side of an instance, which the plug-in reaches through its `user` pointer.
struct HostSide {
    time: std::cell::UnsafeCell<abi::TimeInfo>,
    sample_rate: f64,
    /// The plug-in reported an edit (a parameter moved in its own UI).
    dirty: AtomicBool,
    /// Its parameter names or layout changed: read them again.
    changed: AtomicBool,
}

// The time is written on the thread that processes, before it calls the plug-in, and
// read by the plug-in during that call.
unsafe impl Sync for HostSide {}

fn side_of(effect: *mut AEffect) -> Option<&'static HostSide> {
    if effect.is_null() {
        return None;
    }
    let user = unsafe { (*effect).user };
    (!user.is_null()).then(|| unsafe { &*(user as *const HostSide) })
}

/// Copy `s` into a plug-in's string buffer of `size` bytes.
fn write_str(ptr: *mut c_void, size: usize, s: &str) {
    if ptr.is_null() {
        return;
    }
    let n = s.len().min(size - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), ptr as *mut u8, n);
        *(ptr as *mut u8).add(n) = 0;
    }
}

/// What DECKARD answers when a plug-in asks.
unsafe extern "C" fn host_callback(effect: *mut AEffect, opcode: i32, _index: i32, _value: isize, ptr: *mut c_void, _opt: f32) -> isize {
    use abi::*;
    let side = side_of(effect);
    match opcode {
        HOST_VERSION => 2400,
        HOST_AUTOMATE => {
            if let Some(s) = side {
                s.dirty.store(true, Ordering::Release);
            }
            0
        }
        HOST_GET_TIME => side.map_or(0, |s| s.time.get() as isize),
        HOST_GET_SAMPLE_RATE => side.map_or(44100, |s| s.sample_rate as isize),
        HOST_GET_BLOCK_SIZE => MAX_BLOCK as isize,
        HOST_IO_CHANGED | HOST_UPDATE_DISPLAY => {
            if let Some(s) = side {
                s.changed.store(true, Ordering::Release);
            }
            1
        }
        HOST_GET_VENDOR_STRING => {
            write_str(ptr, 64, "Panic Signals");
            1
        }
        HOST_GET_PRODUCT_STRING => {
            write_str(ptr, 64, "DECKARD");
            1
        }
        HOST_GET_VENDOR_VERSION => 1,
        HOST_CAN_DO => {
            if ptr.is_null() {
                return 0;
            }
            let what = unsafe { CStr::from_ptr(ptr as *const c_char) }.to_bytes();
            let yes: &[&[u8]] = &[b"sendVstEvents", b"sendVstMidiEvent", b"sendVstTimeInfo", b"receiveVstEvents", b"receiveVstMidiEvent"];
            if yes.contains(&what) { 1 } else { -1 }
        }
        // Not recording automation.
        HOST_GET_AUTOMATION_STATE => 1,
        // English.
        HOST_GET_LANGUAGE => 1,
        HOST_WANT_MIDI | HOST_BEGIN_EDIT | HOST_END_EDIT => 1,
        HOST_CURRENT_ID | HOST_IDLE | HOST_GET_CURRENT_PROCESS_LEVEL => 0,
        _ => 0,
    }
}

/// Libraries loaded so far, by path. They stay loaded for the life of the process (a
/// plug-in's code may still run from its own threads after its instances are gone).
static LIBRARIES: Mutex<Option<HashMap<PathBuf, &'static libloading::Library>>> = Mutex::new(None);

/// The shared library inside a plug-in: the file itself, or a macOS bundle's executable.
pub fn library_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return if path.is_file() { Ok(path.to_path_buf()) } else { Err(format!("{} is missing", path.display())) };
    }
    let macos = path.join("Contents/MacOS");
    let stem = path.file_stem().map(|s| s.to_os_string()).unwrap_or_default();
    if macos.join(&stem).is_file() {
        return Ok(macos.join(stem));
    }
    let mut files: Vec<_> =
        std::fs::read_dir(&macos).map_err(|_| format!("{} has no library", path.display()))?.flatten().map(|e| e.path()).collect();
    files.sort();
    files.into_iter().find(|p| p.is_file()).ok_or_else(|| format!("{} has no library", path.display()))
}

/// The plug-in's entry point, loading its library if it isn't yet.
fn entry_of(path: &Path) -> Result<abi::Entry, String> {
    let lib_path = library_path(path)?;
    let mut libs = LIBRARIES.lock().map_err(|_| "plug-in loader unavailable")?;
    let libs = libs.get_or_insert_with(HashMap::new);
    let lib = match libs.get(&lib_path) {
        Some(l) => *l,
        None => {
            let l = unsafe { libloading::Library::new(&lib_path) }.map_err(|e| format!("couldn't load {}: {e}", lib_path.display()))?;
            let l: &'static libloading::Library = Box::leak(Box::new(l));
            libs.insert(lib_path.clone(), l);
            l
        }
    };
    for name in [&b"VSTPluginMain\0"[..], b"main_macho\0", b"main\0"] {
        if let Ok(f) = unsafe { lib.get::<abi::Entry>(name) } {
            return Ok(*f);
        }
    }
    Err(format!("{} is not a VST2 plug-in (no VSTPluginMain)", lib_path.display()))
}

/// A running VST2 plug-in.
pub struct Vst2 {
    effect: *mut AEffect,
    side: Box<HostSide>,
    sample_rate: f64,
    /// Channel buffers of MAX_BLOCK frames, and pointers to them.
    inputs: Vec<Vec<f32>>,
    outputs: Vec<Vec<f32>>,
    in_ptrs: Vec<*mut f32>,
    out_ptrs: Vec<*mut f32>,
    /// MIDI waiting for the next block.
    midi: Box<[abi::MidiEvent; abi::EVENTS]>,
    events: Box<abi::Events>,
    pending: usize,
    params: Vec<ParamInfo>,
    chunks: bool,
    takes_midi: bool,
    /// Parameters modulation drives, with the knob's value.
    modded: Vec<(u32, f32)>,
    clock: i64,
    was_playing: bool,
    held: [bool; 128],
    moved: [bool; 130],
    /// The Maschine chunk last applied.
    pub(crate) chunk: Vec<u8>,
}

// As for VST3: used from any thread, one at a time, under the instance's mutex.
unsafe impl Send for Vst2 {}

/// What a plug-in says about itself, without keeping it running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub vendor: String,
    pub unique_id: u32,
    pub instrument: bool,
}

impl Vst2 {
    /// An instance of the plug-in at `path`, ready to process at `sample_rate`.
    pub fn load(path: &Path, sample_rate: f64) -> Result<Vst2, String> {
        Self::from_entry(entry_of(path)?, sample_rate)
    }

    /// An instance made by the entry point `entry`.
    pub(crate) fn from_entry(entry: abi::Entry, sample_rate: f64) -> Result<Vst2, String> {
        let effect = unsafe { entry(host_callback) };
        if effect.is_null() {
            return Err("the plug-in didn't start".into());
        }
        let e = unsafe { &mut *effect };
        if e.magic != abi::MAGIC || e.dispatcher.is_none() {
            return Err("not a VST2 plug-in".into());
        }
        let side = Box::new(HostSide {
            time: std::cell::UnsafeCell::new(abi::TimeInfo::default()),
            sample_rate,
            dirty: AtomicBool::new(false),
            changed: AtomicBool::new(false),
        });
        e.user = (&*side as *const HostSide) as *mut c_void;
        let mut v = Vst2 {
            effect,
            side,
            sample_rate,
            inputs: Vec::new(),
            outputs: Vec::new(),
            in_ptrs: Vec::new(),
            out_ptrs: Vec::new(),
            midi: Box::new([abi::MidiEvent::default(); abi::EVENTS]),
            events: Box::new(abi::Events { num_events: 0, reserved: 0, events: [std::ptr::null_mut(); abi::EVENTS] }),
            pending: 0,
            params: Vec::new(),
            chunks: false,
            takes_midi: false,
            modded: Vec::with_capacity(MODDED),
            clock: 0,
            was_playing: false,
            held: [false; 128],
            moved: [false; 130],
            chunk: Vec::new(),
        };
        v.dispatch(abi::EFF_OPEN, 0, 0, std::ptr::null_mut(), 0.0);
        if v.dispatch(abi::EFF_GET_PLUG_CATEGORY, 0, 0, std::ptr::null_mut(), 0.0) == abi::CATEGORY_SHELL {
            return Err("plug-in shells (several plug-ins in one) aren't supported".into());
        }
        v.dispatch(abi::EFF_SET_SAMPLE_RATE, 0, 0, std::ptr::null_mut(), sample_rate as f32);
        v.dispatch(abi::EFF_SET_BLOCK_SIZE, 0, MAX_BLOCK as isize, std::ptr::null_mut(), 0.0);
        // 32-bit floats.
        v.dispatch(abi::EFF_SET_PROCESS_PRECISION, 0, 0, std::ptr::null_mut(), 0.0);
        let (flags, ins, outs) = {
            let e = v.aeffect();
            (e.flags, e.num_inputs.clamp(0, 64) as usize, e.num_outputs.clamp(0, 64) as usize)
        };
        v.chunks = flags & abi::FLAG_PROGRAM_CHUNKS != 0;
        v.takes_midi = flags & abi::FLAG_IS_SYNTH != 0 || v.can_do("receiveVstMidiEvent") || v.can_do("receiveVstEvents");
        v.inputs = vec![vec![0f32; MAX_BLOCK]; ins];
        v.outputs = vec![vec![0f32; MAX_BLOCK]; outs];
        v.in_ptrs = v.inputs.iter_mut().map(|b| b.as_mut_ptr()).collect();
        v.out_ptrs = v.outputs.iter_mut().map(|b| b.as_mut_ptr()).collect();
        for (slot, m) in v.events.events.iter_mut().zip(v.midi.iter_mut()) {
            *slot = m;
        }
        v.params = v.read_params();
        v.resume();
        Ok(v)
    }

    fn aeffect(&self) -> &AEffect {
        unsafe { &*self.effect }
    }

    fn dispatch(&self, opcode: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32) -> isize {
        match self.aeffect().dispatcher {
            Some(d) => unsafe { d(self.effect, opcode, index, value, ptr, opt) },
            None => 0,
        }
    }

    fn can_do(&self, what: &str) -> bool {
        let s = std::ffi::CString::new(what).unwrap_or_default();
        self.dispatch(abi::EFF_CAN_DO, 0, 0, s.as_ptr() as *mut c_void, 0.0) > 0
    }

    /// A string the plug-in writes for `opcode` (plug-ins often overrun the documented
    /// sizes, so the buffer is generous).
    fn string(&self, opcode: i32, index: i32) -> String {
        let mut buf = [0u8; 256];
        self.dispatch(opcode, index, 0, buf.as_mut_ptr().cast(), 0.0);
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).trim().to_string()
    }

    fn resume(&self) {
        self.dispatch(abi::EFF_MAINS_CHANGED, 0, 1, std::ptr::null_mut(), 0.0);
        self.dispatch(abi::EFF_START_PROCESS, 0, 0, std::ptr::null_mut(), 0.0);
    }

    fn suspend(&self) {
        self.dispatch(abi::EFF_STOP_PROCESS, 0, 0, std::ptr::null_mut(), 0.0);
        self.dispatch(abi::EFF_MAINS_CHANGED, 0, 0, std::ptr::null_mut(), 0.0);
    }

    fn read_params(&self) -> Vec<ParamInfo> {
        (0..self.aeffect().num_params.max(0))
            .map(|i| {
                let name = self.string(abi::EFF_GET_PARAM_NAME, i);
                let name = if name.is_empty() { format!("Param {}", i + 1) } else { name };
                ParamInfo {
                    id: i as u32,
                    short: name.clone(),
                    title: name,
                    units: self.string(abi::EFF_GET_PARAM_LABEL, i),
                    steps: 0,
                    default: self.get(i as u32) as f64,
                }
            })
            .collect()
    }

    fn get(&self, id: u32) -> f32 {
        match self.aeffect().get_parameter {
            Some(g) if (id as i32) < self.aeffect().num_params => unsafe { g(self.effect, id as i32) },
            _ => 0.0,
        }
    }

    fn set(&self, id: u32, v: f32) {
        if let Some(s) = self.aeffect().set_parameter
            && (id as i32) < self.aeffect().num_params
        {
            unsafe { s(self.effect, id as i32, v.clamp(0.0, 1.0)) };
        }
    }

    /// What the plug-in says about itself.
    pub fn info(&self) -> Info {
        let name = self.string(abi::EFF_GET_EFFECT_NAME, 0);
        let name = if name.is_empty() { self.string(abi::EFF_GET_PRODUCT_STRING, 0) } else { name };
        let synth = self.aeffect().flags & abi::FLAG_IS_SYNTH != 0
            || self.dispatch(abi::EFF_GET_PLUG_CATEGORY, 0, 0, std::ptr::null_mut(), 0.0) == abi::CATEGORY_SYNTH;
        Info { name, vendor: self.string(abi::EFF_GET_VENDOR_STRING, 0), unique_id: self.aeffect().unique_id as u32, instrument: synth }
    }

    /// It has a window of its own (not opened by DECKARD yet).
    pub fn has_editor(&self) -> bool {
        self.aeffect().flags & abi::FLAG_HAS_EDITOR != 0
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// `component` is the plug-in's chunk, or an `.fxp` of its parameters.
    pub fn set_state(&mut self, state: &State) -> Result<(), String> {
        let data = &state.component;
        if data.is_empty() {
            return Ok(());
        }
        self.suspend();
        let r = if data.starts_with(b"CcnK") { self.set_fxp(data) } else { self.set_chunk(data, false) };
        self.resume();
        r
    }

    fn set_chunk(&mut self, data: &[u8], program: bool) -> Result<(), String> {
        if !self.chunks {
            return Err("the plug-in's state doesn't read (it keeps no chunk)".into());
        }
        // The plug-in may keep the pointer only for the call; it reads from our copy.
        let mut copy = data.to_vec();
        self.dispatch(abi::EFF_SET_CHUNK, program as i32, copy.len() as isize, copy.as_mut_ptr().cast(), 0.0);
        Ok(())
    }

    /// A standard `.fxp`/`.fxb`: parameters (`FxCk`) or an opaque chunk (`FPCh`, `FBCh`).
    fn set_fxp(&mut self, data: &[u8]) -> Result<(), String> {
        let be = |at: usize| data.get(at..at + 4).map(|b| i32::from_be_bytes(b.try_into().unwrap()));
        let magic = data.get(8..12).ok_or("the plug-in's state is too short")?;
        match magic {
            b"FxCk" => {
                let n = be(24).ok_or("the plug-in's state is too short")?.max(0) as usize;
                for i in 0..n {
                    let v = be(56 + 4 * i).map(|b| f32::from_bits(b as u32)).ok_or("the plug-in's state is truncated")?;
                    self.set(i as u32, v);
                }
                Ok(())
            }
            b"FPCh" | b"FBCh" => {
                let (size_at, data_at) = if magic == b"FPCh" { (56, 60) } else { (156, 160) };
                let n = be(size_at).ok_or("the plug-in's state is too short")?.max(0) as usize;
                let chunk = data.get(data_at..data_at + n).ok_or("the plug-in's state is truncated")?.to_vec();
                self.set_chunk(&chunk, magic == b"FPCh")
            }
            _ => Err("the plug-in's state isn't a VST2 preset".into()),
        }
    }

    /// The plug-in's chunk (bank form), or its parameters as an `.fxp` program.
    pub fn state(&mut self) -> State {
        if self.chunks {
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let n = self.dispatch(abi::EFF_GET_CHUNK, 0, 0, (&raw mut ptr).cast(), 0.0);
            if n > 0 && !ptr.is_null() {
                let component = unsafe { std::slice::from_raw_parts(ptr, n as usize) }.to_vec();
                return State { component, controller: Vec::new() };
            }
        }
        State { component: self.fxp(), controller: Vec::new() }
    }

    fn fxp(&self) -> Vec<u8> {
        let e = self.aeffect();
        let n = e.num_params.max(0) as usize;
        let mut out = Vec::with_capacity(56 + 4 * n);
        out.extend(b"CcnK");
        out.extend(((48 + 4 * n) as i32).to_be_bytes());
        out.extend(b"FxCk");
        out.extend(1i32.to_be_bytes());
        out.extend(e.unique_id.to_be_bytes());
        out.extend(e.version.to_be_bytes());
        out.extend((n as i32).to_be_bytes());
        out.extend([0u8; 28]);
        for i in 0..n {
            out.extend(self.knob(i as u32).to_bits().to_be_bytes());
        }
        out
    }

    pub fn parameter_list(&self, automatable: bool) -> Vec<ParamInfo> {
        if self.side.changed.swap(false, Ordering::AcqRel) {
            // (Names change with a plug-in's mode; the list is read again on next ask.)
            let fresh = self.read_params();
            return self.filter_params(fresh, automatable);
        }
        self.filter_params(self.params.clone(), automatable)
    }

    fn filter_params(&self, params: Vec<ParamInfo>, automatable: bool) -> Vec<ParamInfo> {
        if !automatable {
            return params;
        }
        let can: Vec<bool> =
            params.iter().map(|p| self.dispatch(abi::EFF_CAN_BE_AUTOMATED, p.id as i32, 0, std::ptr::null_mut(), 0.0) > 0).collect();
        // Many plug-ins never answer: then every parameter counts.
        if !can.iter().any(|&c| c) {
            return params;
        }
        params.into_iter().zip(can).filter(|(_, c)| *c).map(|(p, _)| p).collect()
    }

    /// The knob: what the user set, even while modulation plays another value.
    fn knob(&self, id: u32) -> f32 {
        self.modded.iter().find(|m| m.0 == id).map_or_else(|| self.get(id), |m| m.1)
    }

    pub fn param_value(&self, id: u32) -> f64 {
        self.knob(id) as f64
    }

    /// The plug-in shows only its current values, so another value reads as a percentage.
    pub fn param_text(&self, id: u32, v: f64) -> String {
        if (v as f32 - self.get(id)).abs() < 1e-6 {
            let text = self.string(abi::EFF_GET_PARAM_DISPLAY, id as i32);
            let label = self.string(abi::EFF_GET_PARAM_LABEL, id as i32);
            if !text.is_empty() {
                return if label.is_empty() { text } else { format!("{text} {label}") };
            }
        }
        format!("{:.0}%", v * 100.0)
    }

    pub fn set_param(&mut self, id: u32, v: f64) {
        let v = v.clamp(0.0, 1.0) as f32;
        match self.modded.iter_mut().find(|m| m.0 == id) {
            Some(m) => m.1 = v,
            None => self.set(id, v),
        }
        self.side.dirty.store(true, Ordering::Release);
    }

    pub fn take_dirty(&self) -> bool {
        self.side.dirty.swap(false, Ordering::AcqRel)
    }

    pub fn modulate(&mut self, id: u32, v: f64) {
        if !self.modded.iter().any(|m| m.0 == id) {
            if self.modded.len() >= MODDED {
                return;
            }
            self.modded.push((id, self.get(id)));
        }
        self.set(id, v as f32);
    }

    pub fn unmodulate(&mut self, id: u32) {
        if let Some(i) = self.modded.iter().position(|m| m.0 == id) {
            let (_, knob) = self.modded.remove(i);
            self.set(id, knob);
        }
    }

    pub fn modulated(&self, id: u32) -> Option<f64> {
        self.modded.iter().any(|m| m.0 == id).then(|| self.get(id) as f64)
    }

    fn midi(&mut self, data: [u8; 3]) {
        if !self.takes_midi || self.pending >= abi::EVENTS {
            return;
        }
        self.midi[self.pending] = abi::MidiEvent {
            kind: abi::MIDI_TYPE,
            byte_size: std::mem::size_of::<abi::MidiEvent>() as i32,
            flags: abi::MIDI_REALTIME,
            midi_data: [data[0], data[1], data[2], 0],
            ..Default::default()
        };
        self.pending += 1;
    }

    pub fn note_on(&mut self, pitch: i64, velocity: i64) {
        let pitch = pitch.clamp(0, 127);
        if self.held[pitch as usize] {
            self.note_off(pitch);
        }
        self.held[pitch as usize] = true;
        self.midi([0x90, pitch as u8, velocity.clamp(1, 127) as u8]);
    }

    pub fn note_off(&mut self, pitch: i64) {
        let pitch = pitch.clamp(0, 127);
        if std::mem::take(&mut self.held[pitch as usize]) {
            self.midi([0x80, pitch as u8, 0]);
        }
    }

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

    /// VST2 takes MIDI as MIDI: every controller, if it takes MIDI at all.
    pub fn takes_controller(&self, number: usize) -> bool {
        self.takes_midi && (number <= 127 || number == 129)
    }

    pub fn control_change(&mut self, number: u8, value: u8) {
        let (n, v) = (number.min(127), value.min(127));
        self.moved[n as usize] = v != 0;
        self.midi([0xB0, n, v]);
    }

    pub fn pitch_bend(&mut self, value: u16) {
        let v = value.min(16383);
        self.moved[129] = v != 8192;
        self.midi([0xE0, (v & 0x7F) as u8, (v >> 7) as u8]);
    }

    pub fn has_sidechain(&self) -> bool {
        false
    }

    pub fn process_input_sc(
        &mut self,
        frames: usize,
        transport: &Transport,
        input: Option<(&[f32], &[f32])>,
        _sidechain: Option<(&[f32], &[f32])>,
    ) -> (&[f32], &[f32]) {
        let frames = frames.min(MAX_BLOCK);
        if frames == 0 || self.outputs.is_empty() {
            return (&[], &[]);
        }
        // Input: stereo on the first two channels (mono if it has one), the rest silent.
        for c in &mut self.inputs {
            c[..frames].fill(0.0);
        }
        if let Some((l, r)) = input {
            let n = frames.min(l.len()).min(r.len());
            match self.inputs.len() {
                0 => {}
                1 => {
                    for (o, (a, b)) in self.inputs[0][..n].iter_mut().zip(l.iter().zip(r)) {
                        *o = (a + b) * 0.5;
                    }
                }
                _ => {
                    self.inputs[0][..n].copy_from_slice(&l[..n]);
                    self.inputs[1][..n].copy_from_slice(&r[..n]);
                }
            }
        }
        for c in &mut self.outputs {
            c[..frames].fill(0.0);
        }
        self.set_time(transport);
        if self.pending > 0 {
            self.events.num_events = self.pending as i32;
            let events: *mut abi::Events = &raw mut *self.events;
            self.dispatch(abi::EFF_PROCESS_EVENTS, 0, 0, events.cast(), 0.0);
            self.pending = 0;
        }
        let e = self.aeffect();
        let run = e.process_replacing.filter(|_| e.flags & abi::FLAG_CAN_REPLACING != 0 || e.process.is_none()).or(e.process);
        if let Some(run) = run {
            unsafe { run(self.effect, self.in_ptrs.as_mut_ptr(), self.out_ptrs.as_mut_ptr(), frames as i32) };
        }
        self.clock += frames as i64;
        let last = (self.outputs.len() - 1).min(1);
        (&self.outputs[0][..frames], &self.outputs[last][..frames])
    }

    fn set_time(&mut self, t: &Transport) {
        use abi::*;
        let (num, den) = (t.numerator.max(1), t.denominator.max(1));
        let bar = num as f64 * 4.0 / den as f64;
        let mut flags = TIME_PPQ_VALID | TIME_TEMPO_VALID | TIME_BARS_VALID | TIME_SIG_VALID;
        if t.playing {
            flags |= TIME_PLAYING;
        }
        if t.playing != self.was_playing {
            flags |= TIME_TRANSPORT_CHANGED;
        }
        self.was_playing = t.playing;
        let time = TimeInfo {
            sample_pos: self.clock as f64,
            sample_rate: self.sample_rate,
            ppq_pos: t.quarters,
            tempo: if t.tempo > 0.0 { t.tempo } else { 120.0 },
            bar_start_pos: (t.quarters / bar).floor() * bar,
            time_sig_numerator: num,
            time_sig_denominator: den,
            flags,
            ..TimeInfo::default()
        };
        unsafe { *self.side.time.get() = time };
    }

    pub fn output_bus(&self, bus: usize, frames: usize) -> Option<(&[f32], &[f32])> {
        let l = self.outputs.get(2 * bus)?;
        let r = self.outputs.get(2 * bus + 1).unwrap_or(l);
        let n = frames.min(MAX_BLOCK);
        Some((&l[..n], &r[..n]))
    }

    /// Its outputs as stereo pairs (an odd last channel alone).
    pub fn output_count(&self) -> usize {
        self.outputs.len().div_ceil(2)
    }

    pub fn output_name(&self, bus: usize) -> Option<String> {
        let mut p = abi::PinProperties { label: [0; 64], flags: 0, arrangement_type: 0, short_label: [0; 8], future: [0; 48] };
        if self.dispatch(abi::EFF_GET_OUTPUT_PROPERTIES, (2 * bus) as i32, 0, (&raw mut p).cast(), 0.0) <= 0 {
            return None;
        }
        let end = p.label.iter().position(|&b| b == 0).unwrap_or(p.label.len());
        Some(String::from_utf8_lossy(&p.label[..end]).trim().to_string())
    }
}

impl Drop for Vst2 {
    fn drop(&mut self) {
        self.suspend();
        self.dispatch(abi::EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
    }
}

/// What a plug-in at `path` is, by loading it (do this in a helper process: a plug-in can
/// crash or hang while loading).
pub fn probe(path: &Path) -> Result<Info, String> {
    Vst2::load(path, 44100.0).map(|v| v.info())
}

/// VST2 plug-ins DECKARD may load, by unique id: (id, name, path). The app sets it from
/// its scan, since finding a plug-in's id means loading it.
static INSTALLED: OnceLock<Mutex<Vec<(u32, String, PathBuf)>>> = OnceLock::new();

/// Tell the host which VST2 plug-ins are installed (from the scan).
pub fn set_installed(plugins: Vec<(u32, String, PathBuf)>) {
    let m = INSTALLED.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut l) = m.lock() {
        *l = plugins;
    }
}

/// The installed plug-in with unique id `uid`, preferring one named `name` (two vendors
/// may use the same id).
pub fn find_installed(uid: u32, name: &str) -> Option<PathBuf> {
    let l = INSTALLED.get()?.lock().ok()?;
    let same = |n: &str| n.eq_ignore_ascii_case(name.trim());
    l.iter().find(|(u, n, _)| *u == uid && same(n)).or_else(|| l.iter().find(|(u, _, _)| *u == uid)).map(|(_, _, p)| p.clone())
}

/// The binary an MPC plug-in folder has for this machine (`file64` on 64-bit ARM Linux,
/// `file32` on 32-bit), with what its `plugin.json` says.
pub fn mpc_folder(dir: &Path) -> Option<(Info, PathBuf)> {
    let text = std::fs::read_to_string(dir.join("plugin.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "aarch64") => "file64",
        ("linux", "arm") => "file32",
        _ => return None,
    };
    let file = dir.join(v.get(key)?.as_str()?);
    let uid = mpc_uid(v.get("uid")?)?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let info = Info { name: s("name"), vendor: s("manufacturer"), unique_id: uid, instrument: s("kind") == "instrument" };
    file.is_file().then_some((info, file))
}

/// `plugin.json`'s `uid`: a four-character code, a number, or `"0x…"` hex.
fn mpc_uid(v: &serde_json::Value) -> Option<u32> {
    if let Some(n) = v.as_u64() {
        return u32::try_from(n).ok();
    }
    let s = v.as_str()?;
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if s.len() == 4 && s.is_ascii() {
        return Some(u32::from_be_bytes(s.as_bytes().try_into().ok()?));
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A tiny plug-in, as a library would export it: a synth with two parameters that
    /// plays a constant per held note (gain × notes) and keeps its state as a chunk
    /// ([`entry`]) or as parameters ([`entry_plain`]).
    #[repr(C)]
    struct Fake {
        effect: AEffect,
        params: [f32; 2],
        notes: i32,
        last_events: i32,
        tempo: f64,
        chunk: [u8; 8],
    }

    static CLOSED: AtomicUsize = AtomicUsize::new(0);

    fn fake(e: *mut AEffect) -> &'static mut Fake {
        unsafe { &mut *(e as *mut Fake) }
    }

    unsafe extern "C" fn dispatcher(e: *mut AEffect, op: i32, index: i32, value: isize, ptr: *mut c_void, _opt: f32) -> isize {
        let f = fake(e);
        match op {
            abi::EFF_GET_PARAM_NAME => {
                write_str(ptr, 64, ["Gain", "Tone"][index as usize]);
                1
            }
            abi::EFF_GET_PARAM_DISPLAY => {
                write_str(ptr, 64, &format!("{:.1}", f.params[index as usize] * 10.0));
                1
            }
            abi::EFF_GET_PARAM_LABEL => {
                write_str(ptr, 64, if index == 0 { "dB" } else { "" });
                1
            }
            abi::EFF_GET_EFFECT_NAME => {
                write_str(ptr, 64, "Fake Synth");
                1
            }
            abi::EFF_GET_VENDOR_STRING => {
                write_str(ptr, 64, "Acme");
                1
            }
            abi::EFF_PROCESS_EVENTS => {
                let evs = unsafe { &*(ptr as *const abi::Events) };
                f.last_events = evs.num_events;
                for i in 0..evs.num_events as usize {
                    let m = unsafe { &*evs.events[i] };
                    match m.midi_data[0] & 0xF0 {
                        0x90 => f.notes += 1,
                        0x80 => f.notes -= 1,
                        _ => {}
                    }
                }
                1
            }
            abi::EFF_GET_CHUNK => {
                // (The plug-in keeps the chunk until it's asked again.)
                let b = &mut f.chunk;
                b[..4].copy_from_slice(&f.params[0].to_le_bytes());
                b[4..].copy_from_slice(&f.params[1].to_le_bytes());
                unsafe { *(ptr as *mut *mut u8) = b.as_mut_ptr() };
                8
            }
            abi::EFF_SET_CHUNK => {
                let d = unsafe { std::slice::from_raw_parts(ptr as *const u8, value as usize) };
                f.params[0] = f32::from_le_bytes(d[..4].try_into().unwrap());
                f.params[1] = f32::from_le_bytes(d[4..8].try_into().unwrap());
                1
            }
            abi::EFF_CAN_BE_AUTOMATED => (index == 0) as isize,
            abi::EFF_CLOSE => {
                CLOSED.fetch_add(1, Ordering::SeqCst);
                drop(unsafe { Box::from_raw(e as *mut Fake) });
                0
            }
            _ => 0,
        }
    }

    unsafe extern "C" fn set_parameter(e: *mut AEffect, i: i32, v: f32) {
        fake(e).params[i as usize] = v;
    }

    unsafe extern "C" fn get_parameter(e: *mut AEffect, i: i32) -> f32 {
        fake(e).params[i as usize]
    }

    unsafe extern "C" fn process_replacing(e: *mut AEffect, _ins: *mut *mut f32, outs: *mut *mut f32, n: i32) {
        let f = fake(e);
        let time = unsafe { abi_host(e, abi::HOST_GET_TIME) } as *const abi::TimeInfo;
        if !time.is_null() {
            f.tempo = unsafe { (*time).tempo };
        }
        for c in 0..2 {
            let out = unsafe { std::slice::from_raw_parts_mut(*outs.add(c), n as usize) };
            out.fill(f.params[0] * f.notes as f32);
        }
    }

    static HOST: OnceLock<abi::HostCallback> = OnceLock::new();

    unsafe fn abi_host(e: *mut AEffect, op: i32) -> isize {
        unsafe { HOST.get().unwrap()(e, op, 0, 0, std::ptr::null_mut(), 0.0) }
    }

    unsafe extern "C" fn entry(host: abi::HostCallback) -> *mut AEffect {
        make(host, true)
    }

    unsafe extern "C" fn entry_plain(host: abi::HostCallback) -> *mut AEffect {
        make(host, false)
    }

    fn make(host: abi::HostCallback, chunks: bool) -> *mut AEffect {
        let _ = HOST.set(host);
        assert_eq!(unsafe { host(std::ptr::null_mut(), abi::HOST_VERSION, 0, 0, std::ptr::null_mut(), 0.0) }, 2400);
        let flags = abi::FLAG_CAN_REPLACING | abi::FLAG_IS_SYNTH | if chunks { abi::FLAG_PROGRAM_CHUNKS } else { 0 };
        let f = Box::new(Fake {
            effect: AEffect {
                magic: abi::MAGIC,
                dispatcher: Some(dispatcher),
                process: None,
                set_parameter: Some(set_parameter),
                get_parameter: Some(get_parameter),
                num_programs: 1,
                num_params: 2,
                num_inputs: 0,
                num_outputs: 2,
                flags,
                resvd1: 0,
                resvd2: 0,
                initial_delay: 0,
                real_qualities: 0,
                off_qualities: 0,
                io_ratio: 1.0,
                object: std::ptr::null_mut(),
                user: std::ptr::null_mut(),
                unique_id: i32::from_be_bytes(*b"AcFk"),
                version: 1,
                process_replacing: Some(process_replacing),
                process_double_replacing: None,
                future: [0; 56],
            },
            params: [0.5, 0.25],
            notes: 0,
            last_events: 0,
            tempo: 0.0,
            chunk: [0; 8],
        });
        // (`effect` is the first field, so the plug-in finds itself from the pointer.)
        Box::into_raw(f) as *mut AEffect
    }

    fn transport() -> Transport {
        Transport { tempo: 98.0, playing: true, quarters: 5.0, numerator: 4, denominator: 4 }
    }

    #[test]
    fn hosts_a_synth_notes_params_and_time() {
        let mut v = Vst2::from_entry(entry, 48000.0).unwrap();
        let info = v.info();
        assert_eq!((info.name.as_str(), info.vendor.as_str(), info.instrument), ("Fake Synth", "Acme", true));
        assert_eq!(info.unique_id, u32::from_be_bytes(*b"AcFk"));
        assert_eq!(v.parameter_list(false).iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["Gain", "Tone"]);
        // Only Gain says it can be automated.
        assert_eq!(v.parameter_list(true).len(), 1);
        assert_eq!(v.param_text(0, 0.5), "5.0 dB");
        assert_eq!(v.param_text(0, 0.2), "20%");
        // Silent until a note; then gain × notes, with the tempo passed on.
        let (l, _) = v.process(64);
        assert!(l.iter().all(|&s| s == 0.0));
        v.note_on(60, 100);
        v.note_on(64, 100);
        let (l, r) = v.process_input_sc(64, &transport(), None, None);
        assert_eq!((l[0], r[63]), (1.0, 1.0));
        assert_eq!(fake(v.effect).last_events, 2);
        assert_eq!(fake(v.effect).tempo, 98.0);
        v.note_off(64);
        assert_eq!(v.process(64).0[0], 0.5);
        // A knob counts as an edit; modulation plays a value and lets the knob back.
        assert!(!v.take_dirty());
        v.set_param(0, 1.0);
        assert!(v.take_dirty());
        v.modulate(0, 0.25);
        assert_eq!((v.param_value(0), v.modulated(0)), (1.0, Some(0.25)));
        v.set_param(0, 0.75);
        v.unmodulate(0);
        assert_eq!(v.param_value(0), 0.75);
        // Its state is its chunk, and reads back.
        let state = v.state();
        v.set_param(0, 0.0);
        v.set_state(&state).unwrap();
        assert_eq!(v.param_value(0), 0.75);
        let closed = CLOSED.load(Ordering::SeqCst);
        drop(v);
        assert_eq!(CLOSED.load(Ordering::SeqCst), closed + 1);
    }

    #[test]
    fn a_plugin_without_chunks_saves_an_fxp() {
        let mut v = Vst2::from_entry(entry_plain, 44100.0).unwrap();
        v.set_param(1, 0.875);
        let state = v.state();
        assert_eq!(&state.component[..4], b"CcnK");
        assert_eq!(&state.component[8..12], b"FxCk");
        v.set_param(1, 0.0);
        v.set_state(&state).unwrap();
        assert_eq!(v.param_value(1), 0.875);
    }

    impl Vst2 {
        fn process(&mut self, frames: usize) -> (&[f32], &[f32]) {
            self.process_input_sc(frames, &Transport::default(), None, None)
        }
    }

    /// Real plug-ins, when given: `DECKARD_VST2_SYNTH` (an instrument) and
    /// `DECKARD_VST2_FX` (an effect), paths to a `.vst`, `.so` or `.dll`.
    #[test]
    fn real_plugins_play_and_keep_their_state() {
        if let Some(path) = std::env::var_os("DECKARD_VST2_SYNTH") {
            let path = PathBuf::from(path);
            let info = probe(&path).unwrap();
            assert!(info.instrument, "{info:?}");
            let mut v = Vst2::load(&path, 48000.0).unwrap();
            assert!(!v.parameter_list(false).is_empty());
            v.note_on(60, 110);
            let loud = (0..20).map(|_| v.process(256).0.iter().fold(0f32, |m, s| m.max(s.abs()))).fold(0f32, f32::max);
            assert!(loud > 0.01, "the synth plays: peak {loud}");
            v.note_off(60);
            let p = v.parameter_list(false)[0].id;
            v.set_param(p, 0.3);
            let state = v.state();
            v.set_param(p, 0.9);
            v.set_state(&state).unwrap();
            assert!((v.param_value(p) - 0.3).abs() < 1e-3, "state restores: {}", v.param_value(p));
            eprintln!("{info:?}: peak {loud}, {} parameters", v.parameter_list(false).len());
        }
        if let Some(path) = std::env::var_os("DECKARD_VST2_FX") {
            let path = PathBuf::from(path);
            let mut v = Vst2::load(&path, 48000.0).unwrap();
            let impulse: Vec<f32> = (0..256).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect();
            let silence = vec![0f32; 256];
            let mut heard = 0f32;
            for block in 0..200 {
                let input = if block == 0 { &impulse } else { &silence };
                let (l, _) = v.process_input_sc(256, &Transport { tempo: 120.0, ..Transport::default() }, Some((input, input)), None);
                heard = heard.max(l.iter().fold(0f32, |m, s| m.max(s.abs())));
            }
            assert!(heard > 0.01, "the effect passes its input: peak {heard}");
            eprintln!("{:?}: peak {heard}", v.info());
        }
    }

    #[test]
    fn mpc_plugin_ids() {
        assert_eq!(mpc_uid(&serde_json::json!("AcWb")), Some(u32::from_be_bytes(*b"AcWb")));
        assert_eq!(mpc_uid(&serde_json::json!(1234)), Some(1234));
        assert_eq!(mpc_uid(&serde_json::json!("0x41634662")), Some(0x4163_4662));
        assert_eq!(mpc_uid(&serde_json::json!("12345")), Some(12345));
    }

    #[test]
    fn an_mpc_folder_is_read_without_loading() {
        let dir = std::env::temp_dir().join(format!("deckard-mpc-{}", std::process::id())).join("Acme - VST - Fake");
        std::fs::create_dir_all(dir.join("VST")).unwrap();
        std::fs::write(dir.join("VST/fake-arm64.so"), b"").unwrap();
        std::fs::write(dir.join("VST/fake-arm32.so"), b"").unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            r#"{"name":"Fake","manufacturer":"Acme","kind":"instrument","uid":"AcFk","file32":"VST/fake-arm32.so","file64":"VST/fake-arm64.so"}"#,
        )
        .unwrap();
        let found = mpc_folder(&dir);
        if cfg!(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm"))) {
            let (info, file) = found.unwrap();
            assert_eq!((info.name.as_str(), info.vendor.as_str(), info.instrument), ("Fake", "Acme", true));
            assert_eq!(info.unique_id, u32::from_be_bytes(*b"AcFk"));
            assert!(file.ends_with(if cfg!(target_arch = "aarch64") { "fake-arm64.so" } else { "fake-arm32.so" }));
        } else {
            // Not a machine MPC binaries run on.
            assert!(found.is_none());
        }
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn installed_plugins_are_found_by_id_then_name() {
        let a = PathBuf::from("/a.so");
        let b = PathBuf::from("/b.so");
        set_installed(vec![(7, "One".into(), a.clone()), (7, "Two".into(), b.clone())]);
        assert_eq!(find_installed(7, "two"), Some(b));
        assert_eq!(find_installed(7, "Other"), Some(a));
        assert_eq!(find_installed(8, "One"), None);
    }
}
