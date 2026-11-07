//! Hosting Audio Units (macOS), as Maschine does: AU v2 calls, on a component found in
//! the system's registry (`docs/plugins-hosting.md`, sections 1.1 to 1.3).
//!
//! An instance is the same to the rest of the app as a VST3: it's made on the main thread,
//! and the audio thread renders it under the instance's mutex. What differs from VST3:
//! one object holds both the sound and the parameters, so a parameter has one value. To
//! keep the knob apart from what modulation plays, [`Shared`] remembers the knob of each
//! parameter modulation drives.

use crate::state::{self, State};
use crate::{MAX_BLOCK, MODDED, ParamInfo, Transport};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

type Unit = *mut c_void;
type Cfref = *const c_void;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Desc {
    kind: u32,
    sub: u32,
    manufacturer: u32,
    flags: u32,
    mask: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Asbd {
    rate: f64,
    format: u32,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Buffer {
    channels: u32,
    bytes: u32,
    data: *mut c_void,
}

/// An `AudioBufferList` with room for two buffers.
#[repr(C)]
struct BufferList {
    count: u32,
    buffers: [Buffer; 2],
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Smpte {
    subframes: i16,
    divisor: i16,
    counter: u32,
    kind: u32,
    flags: u32,
    hours: i16,
    minutes: i16,
    seconds: i16,
    frames: i16,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct TimeStamp {
    sample_time: f64,
    host_time: u64,
    rate_scalar: f64,
    word_clock: u64,
    smpte: Smpte,
    flags: u32,
    reserved: u32,
}

type RenderProc = unsafe extern "C" fn(*mut c_void, *mut u32, *const TimeStamp, u32, u32, *mut BufferList) -> i32;

#[repr(C)]
struct RenderCallback {
    proc_: RenderProc,
    refcon: *mut c_void,
}

#[repr(C)]
struct HostCallbacks {
    user: *mut c_void,
    beat_and_tempo: unsafe extern "C" fn(*mut c_void, *mut f64, *mut f64) -> i32,
    musical_time: unsafe extern "C" fn(*mut c_void, *mut u32, *mut f32, *mut u32, *mut f64) -> i32,
    transport: unsafe extern "C" fn(*mut c_void, *mut u8, *mut u8, *mut f64, *mut u8, *mut f64, *mut f64) -> i32,
    transport2: unsafe extern "C" fn(*mut c_void, *mut u8, *mut u8, *mut u8, *mut f64, *mut u8, *mut f64, *mut f64) -> i32,
}

#[repr(C)]
struct RawParamInfo {
    name: [c_char; 52],
    unit_name: Cfref,
    clump: u32,
    cf_name: Cfref,
    unit: u32,
    min: f32,
    max: f32,
    default: f32,
    flags: u32,
}

#[repr(C)]
struct StringFromValue {
    id: u32,
    value: *const f32,
    out: Cfref,
}

/// `AudioUnitParameter`, and `AudioUnitProperty` (the same layout).
#[repr(C)]
#[derive(Clone, Copy)]
struct Target {
    unit: Unit,
    id: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
struct AuEvent {
    kind: u32,
    target: Target,
}

type ListenerProc = unsafe extern "C" fn(*mut c_void, *mut c_void, *const AuEvent, u64, f32);

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(after: *mut c_void, desc: *const Desc) -> *mut c_void;
    fn AudioComponentGetDescription(c: *mut c_void, desc: *mut Desc) -> i32;
    fn AudioComponentCopyName(c: *mut c_void, name: *mut Cfref) -> i32;
    fn AudioComponentInstanceNew(c: *mut c_void, out: *mut Unit) -> i32;
    fn AudioComponentInstanceDispose(unit: Unit) -> i32;
    fn AudioUnitInitialize(unit: Unit) -> i32;
    fn AudioUnitUninitialize(unit: Unit) -> i32;
    fn AudioUnitReset(unit: Unit, scope: u32, element: u32) -> i32;
    fn AudioUnitGetPropertyInfo(unit: Unit, id: u32, scope: u32, element: u32, size: *mut u32, writable: *mut u8) -> i32;
    fn AudioUnitGetProperty(unit: Unit, id: u32, scope: u32, element: u32, data: *mut c_void, size: *mut u32) -> i32;
    fn AudioUnitSetProperty(unit: Unit, id: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> i32;
    fn AudioUnitGetParameter(unit: Unit, id: u32, scope: u32, element: u32, value: *mut f32) -> i32;
    fn AudioUnitSetParameter(unit: Unit, id: u32, scope: u32, element: u32, value: f32, offset: u32) -> i32;
    fn AudioUnitRender(unit: Unit, flags: *mut u32, ts: *const TimeStamp, bus: u32, frames: u32, data: *mut BufferList) -> i32;
    fn MusicDeviceMIDIEvent(unit: Unit, status: u32, d1: u32, d2: u32, offset: u32) -> i32;
    fn AUEventListenerCreate(
        proc_: ListenerProc,
        user: *mut c_void,
        run_loop: *const c_void,
        mode: Cfref,
        interval: f32,
        granularity: f32,
        out: *mut *mut c_void,
    ) -> i32;
    fn AUEventListenerAddEventType(listener: *mut c_void, object: *mut c_void, event: *const AuEvent) -> i32;
    fn AUListenerDispose(listener: *mut c_void) -> i32;
    fn AUParameterSet(listener: *mut c_void, object: *mut c_void, target: *const Target, value: f32, offset: u32) -> i32;
    fn AUParameterListenerNotify(listener: *mut c_void, object: *mut c_void, target: *const Target) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: Cfref;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRunLoopGetMain() -> *const c_void;
    fn CFRelease(cf: Cfref);
    fn CFStringGetCString(s: Cfref, buf: *mut c_char, len: isize, enc: u32) -> u8;
    fn CFStringCreateWithCString(alloc: Cfref, s: *const c_char, enc: u32) -> Cfref;
    fn CFArrayGetCount(a: Cfref) -> isize;
    fn CFArrayGetValueAtIndex(a: Cfref, i: isize) -> Cfref;
    fn CFDataCreate(alloc: Cfref, bytes: *const u8, len: isize) -> Cfref;
    fn CFDataGetBytePtr(d: Cfref) -> *const u8;
    fn CFDataGetLength(d: Cfref) -> isize;
    fn CFDictionaryCreateMutable(alloc: Cfref, cap: isize, keys: *const c_void, values: *const c_void) -> *mut c_void;
    fn CFDictionarySetValue(d: *mut c_void, k: Cfref, v: Cfref);
    fn CFDictionaryGetValue(d: Cfref, k: Cfref) -> Cfref;
    fn CFNumberCreate(alloc: Cfref, kind: isize, value: *const c_void) -> Cfref;
    fn CFPropertyListCreateData(alloc: Cfref, plist: Cfref, format: isize, options: usize, error: *mut Cfref) -> Cfref;
    fn CFPropertyListCreateWithData(alloc: Cfref, data: Cfref, options: usize, format: *mut isize, error: *mut Cfref) -> Cfref;
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_msgSend();
}

const UTF8: u32 = 0x0800_0100;
const GLOBAL: u32 = 0;
const INPUT: u32 = 1;
const OUTPUT: u32 = 2;

const PROP_CLASS_INFO: u32 = 0;
const PROP_PARAMETER_LIST: u32 = 3;
const PROP_PARAMETER_INFO: u32 = 4;
const PROP_STREAM_FORMAT: u32 = 8;
const PROP_ELEMENT_COUNT: u32 = 11;
const PROP_MAX_FRAMES: u32 = 14;
const PROP_VALUE_STRINGS: u32 = 16;
const PROP_RENDER_CALLBACK: u32 = 23;
const PROP_HOST_CALLBACKS: u32 = 27;
const PROP_COCOA_UI: u32 = 31;
const PROP_STRING_FROM_VALUE: u32 = 33;
const PROP_PRESENT_PRESET: u32 = 36;

const ANY_PARAMETER: u32 = 0xFFFF_FFFF;
const EVENT_PARAMETER_CHANGED: u32 = 0;
const EVENT_GESTURE_END: u32 = 2;
const EVENT_PROPERTY_CHANGED: u32 = 3;

const FLAG_CF_NAME_RELEASE: u32 = 1 << 4;
const FLAG_METER: u32 = 1 << 15;
const FLAG_HAS_STRINGS: u32 = 1 << 21;
const FLAG_HAS_CF_NAME: u32 = 1 << 27;
const FLAG_READABLE: u32 = 1 << 30;
const FLAG_WRITABLE: u32 = 1 << 31;

const UNIT_INDEXED: u32 = 1;
const UNIT_BOOLEAN: u32 = 2;
const UNIT_METERS: u32 = 19;
const UNIT_CUSTOM: u32 = 26;

const RENDER_OUTPUT_IS_SILENCE: u32 = 1 << 4;
const CHUNK_RAW: u32 = 1;
const CHUNK_PLIST: u32 = 3;
const CHUNK_RAW_ALSO: u32 = 4;
const NI: u32 = u32::from_be_bytes(*b"-NI-");
const XML_PLIST: isize = 100;

/// A CoreFoundation object we own.
struct Cf(Cfref);

impl Cf {
    fn new(r: Cfref) -> Option<Cf> {
        (!r.is_null()).then(|| Cf(r))
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

fn cf_string(s: &str) -> Cf {
    let c = CString::new(s).unwrap_or_default();
    Cf(unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) })
}

fn string_of(s: Cfref) -> String {
    if s.is_null() {
        return String::new();
    }
    let mut buf = [0 as c_char; 512];
    if unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) } == 0 {
        return String::new();
    }
    unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

fn cf_i32(v: i32) -> Cf {
    const SINT32: isize = 3;
    Cf(unsafe { CFNumberCreate(std::ptr::null(), SINT32, (&v as *const i32).cast()) })
}

fn bytes_of_data(d: Cfref) -> Vec<u8> {
    let (p, n) = unsafe { (CFDataGetBytePtr(d), CFDataGetLength(d)) };
    if p.is_null() || n <= 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec() }
}

/// What's parsed of a parameter when the plug-in is made, and its knob while modulated.
struct Meta {
    info: ParamInfo,
    min: f32,
    max: f32,
    flags: u32,
    /// The value names of a parameter that has them, from `min` on.
    names: Vec<String>,
    /// The knob's value (normalized) while modulation plays another: [`NOT_MODDED`] else.
    knob: AtomicU64,
    /// What modulation plays (normalized).
    played: AtomicU64,
}

const NOT_MODDED: u64 = u64::MAX;

impl Meta {
    fn plain(&self, v: f64) -> f32 {
        let p = self.min as f64 + v.clamp(0.0, 1.0) * (self.max - self.min) as f64;
        (if self.info.steps > 0 { p.round() } else { p }) as f32
    }

    fn normalized(&self, plain: f32) -> f64 {
        if self.max > self.min { ((plain - self.min) as f64 / (self.max - self.min) as f64).clamp(0.0, 1.0) } else { 0.0 }
    }

    fn knob(&self) -> Option<f64> {
        let k = self.knob.load(Ordering::Acquire);
        (k != NOT_MODDED).then(|| f64::from_bits(k))
    }
}

/// What an instance shares with its window and with the UI's handles on its parameters.
/// The unit lives until the last of them lets go. `AudioUnitGetParameter` and
/// `AudioUnitSetParameter` are made to be called alongside rendering.
pub struct Shared {
    unit: Unit,
    /// What the plug-in reads through its callbacks (its window may, at any time): lives
    /// as long as the unit.
    context: *mut RenderContext,
    params: Vec<Meta>,
    index: HashMap<u32, usize>,
    dirty: AtomicBool,
    /// Its window has been open: not every plug-in reports its window's edits, so the
    /// saved state is compared from then on.
    window_seen: AtomicBool,
    listener: AtomicPtr<c_void>,
}

// See `Shared`: the calls used without the instance's mutex are the ones made for that.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Drop for Shared {
    fn drop(&mut self) {
        unsafe {
            let listener = self.listener.load(Ordering::Acquire);
            if !listener.is_null() {
                AUListenerDispose(listener);
            }
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
            drop(Box::from_raw(self.context));
        }
    }
}

impl Shared {
    fn meta(&self, id: u32) -> Option<&Meta> {
        self.params.get(*self.index.get(&id)?)
    }

    fn target(&self, id: u32) -> Target {
        Target { unit: self.unit, id, scope: GLOBAL, element: 0 }
    }

    fn played_value(&self, id: u32) -> f32 {
        let mut v = 0f32;
        unsafe { AudioUnitGetParameter(self.unit, id, GLOBAL, 0, &mut v) };
        v
    }

    /// A parameter's knob (normalized 0..1).
    pub fn value(&self, id: u32) -> f64 {
        let Some(m) = self.meta(id) else { return 0.0 };
        m.knob().unwrap_or_else(|| m.normalized(self.played_value(id)))
    }

    /// How the plug-in shows value `v` of parameter `id`.
    pub fn text(&self, id: u32, v: f64) -> String {
        let Some(m) = self.meta(id) else { return String::new() };
        let plain = m.plain(v);
        if let Some(name) = m.names.get((plain - m.min).round().max(0.0) as usize) {
            return name.clone();
        }
        if m.flags & FLAG_HAS_STRINGS != 0 {
            let mut q = StringFromValue { id, value: &plain, out: std::ptr::null() };
            let mut size = std::mem::size_of::<StringFromValue>() as u32;
            let r = unsafe { AudioUnitGetProperty(self.unit, PROP_STRING_FROM_VALUE, GLOBAL, 0, (&raw mut q).cast(), &mut size) };
            if let (0, Some(s)) = (r, Cf::new(q.out)) {
                let text = string_of(s.0);
                if !text.is_empty() {
                    return text;
                }
            }
        }
        let number = if m.info.steps > 0 { format!("{plain:.0}") } else { format!("{plain:.2}") };
        if m.info.units.is_empty() { number } else { format!("{number} {}", m.info.units) }
    }

    /// Change a knob from the host: the plug-in and its window take it, unless modulation
    /// is driving it (then it plays when modulation lets go). Counts as an edit.
    pub fn set(&self, id: u32, v: f64) {
        let Some(m) = self.meta(id) else { return };
        let v = v.clamp(0.0, 1.0);
        if m.knob().is_some() {
            m.knob.store(v.to_bits(), Ordering::Release);
        } else {
            let target = self.target(id);
            unsafe { AUParameterSet(self.listener.load(Ordering::Acquire), std::ptr::null_mut(), &target, m.plain(v), 0) };
        }
        self.dirty.store(true, Ordering::Release);
    }

    /// The window and anything else listening should read every parameter again.
    fn notify_all(&self) {
        let target = self.target(ANY_PARAMETER);
        unsafe { AUParameterListenerNotify(self.listener.load(Ordering::Acquire), std::ptr::null_mut(), &target) };
    }
}

/// The plug-in reported an edit (on the main run loop).
unsafe extern "C" fn edited(user: *mut c_void, _object: *mut c_void, _event: *const AuEvent, _time: u64, _value: f32) {
    let shared = unsafe { &*(user as *const Shared) };
    shared.dirty.store(true, Ordering::Release);
}

/// What the callbacks into the host read during a render.
struct RenderContext {
    input: [Vec<f32>; 2],
    input_channels: usize,
    sidechain: [Vec<f32>; 2],
    sc_channels: usize,
    sample_rate: f64,
    tempo: f64,
    quarters: f64,
    playing: bool,
    changed: bool,
    numerator: i32,
    denominator: i32,
}

/// The plug-in asks for its input.
unsafe extern "C" fn give_input(
    user: *mut c_void,
    _flags: *mut u32,
    _ts: *const TimeStamp,
    bus: u32,
    frames: u32,
    list: *mut BufferList,
) -> i32 {
    if list.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *(user as *mut RenderContext) };
    let frames = (frames as usize).min(MAX_BLOCK);
    let count = unsafe { (*list).count as usize };
    // (The list holds as many buffers as the unit asked for.)
    let buffers = unsafe { std::ptr::addr_of_mut!((*list).buffers) as *mut Buffer };
    let (src, ch) = if bus >= 1 { (&mut ctx.sidechain, ctx.sc_channels) } else { (&mut ctx.input, ctx.input_channels) };
    for i in 0..count {
        let b = unsafe { &mut *buffers.add(i) };
        let from = src[i.min(ch.max(1) - 1).min(1)].as_mut_ptr();
        if b.data.is_null() {
            b.data = from.cast();
        } else {
            unsafe { std::ptr::copy_nonoverlapping(from, b.data as *mut f32, frames) };
        }
        b.bytes = (frames * std::mem::size_of::<f32>()) as u32;
    }
    0
}

unsafe fn put<T>(p: *mut T, v: T) {
    if !p.is_null() {
        unsafe { *p = v };
    }
}

unsafe extern "C" fn beat_and_tempo(user: *mut c_void, beat: *mut f64, tempo: *mut f64) -> i32 {
    let ctx = unsafe { &*(user as *const RenderContext) };
    unsafe {
        put(beat, ctx.quarters);
        put(tempo, ctx.tempo);
    }
    0
}

unsafe extern "C" fn musical_time(
    user: *mut c_void,
    to_next: *mut u32,
    numerator: *mut f32,
    denominator: *mut u32,
    down_beat: *mut f64,
) -> i32 {
    let ctx = unsafe { &*(user as *const RenderContext) };
    let bar = (ctx.numerator.max(1) as f64 * 4.0 / ctx.denominator.max(1) as f64).max(f64::MIN_POSITIVE);
    let to_beat = (ctx.quarters.ceil() - ctx.quarters) * 60.0 / ctx.tempo * ctx.sample_rate;
    unsafe {
        put(to_next, to_beat.max(0.0) as u32);
        put(numerator, ctx.numerator.max(1) as f32);
        put(denominator, ctx.denominator.max(1) as u32);
        put(down_beat, (ctx.quarters / bar).floor() * bar);
    }
    0
}

unsafe extern "C" fn transport_state(
    user: *mut c_void,
    playing: *mut u8,
    changed: *mut u8,
    sample: *mut f64,
    cycling: *mut u8,
    start: *mut f64,
    end: *mut f64,
) -> i32 {
    unsafe { transport_state2(user, playing, std::ptr::null_mut(), changed, sample, cycling, start, end) }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn transport_state2(
    user: *mut c_void,
    playing: *mut u8,
    recording: *mut u8,
    changed: *mut u8,
    sample: *mut f64,
    cycling: *mut u8,
    start: *mut f64,
    end: *mut f64,
) -> i32 {
    let ctx = unsafe { &*(user as *const RenderContext) };
    unsafe {
        put(playing, ctx.playing as u8);
        put(recording, 0);
        put(changed, ctx.changed as u8);
        put(sample, ctx.quarters * 60.0 / ctx.tempo * ctx.sample_rate);
        put(cycling, 0);
        put(start, 0.0);
        put(end, 0.0);
    }
    0
}

/// The installed component of type `kind` and subtype `sub`: of `vendor` if one is, or
/// the only one there is.
fn find_component(kind: u32, sub: u32, vendor: &str) -> Option<*mut c_void> {
    let want = Desc { kind, sub, ..Desc::default() };
    let mut found = Vec::new();
    let mut c = std::ptr::null_mut();
    loop {
        c = unsafe { AudioComponentFindNext(c, &want) };
        if c.is_null() {
            break;
        }
        let mut name = std::ptr::null();
        let full = if unsafe { AudioComponentCopyName(c, &mut name) } == 0 {
            let s = Cf::new(name).map_or(String::new(), |n| string_of(n.0));
            s.split_once(": ").map_or(String::new(), |(v, _)| v.to_string())
        } else {
            String::new()
        };
        if full == vendor {
            return Some(c);
        }
        found.push(c);
    }
    (found.len() == 1).then(|| found[0])
}

fn stream_format(rate: f64, channels: u32) -> Asbd {
    const LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
    const FLOAT_PACKED_NON_INTERLEAVED: u32 = 1 | 8 | 32;
    Asbd {
        rate,
        format: LINEAR_PCM,
        flags: FLOAT_PACKED_NON_INTERLEAVED,
        bytes_per_packet: 4,
        frames_per_packet: 1,
        bytes_per_frame: 4,
        channels,
        bits: 32,
        reserved: 0,
    }
}

fn set_format(unit: Unit, scope: u32, element: u32, format: &Asbd) -> i32 {
    unsafe {
        AudioUnitSetProperty(unit, PROP_STREAM_FORMAT, scope, element, (format as *const Asbd).cast(), std::mem::size_of::<Asbd>() as u32)
    }
}

fn get_u32(unit: Unit, id: u32, scope: u32, element: u32) -> Option<u32> {
    let mut v = 0u32;
    let mut size = 4u32;
    let r = unsafe { AudioUnitGetProperty(unit, id, scope, element, (&raw mut v).cast(), &mut size) };
    (r == 0).then_some(v)
}

fn channels_of(unit: Unit, scope: u32, element: u32) -> Option<usize> {
    let mut f = Asbd::default();
    let mut size = std::mem::size_of::<Asbd>() as u32;
    let r = unsafe { AudioUnitGetProperty(unit, PROP_STREAM_FORMAT, scope, element, (&raw mut f).cast(), &mut size) };
    (r == 0).then_some(f.channels as usize)
}

fn unit_label(unit: u32) -> &'static str {
    match unit {
        3 => "%",
        4 => "s",
        5 => "samples",
        6 | 15 => "°",
        8 => "Hz",
        9 | 20 => "ct",
        10 => "st",
        13 => "dB",
        21 => "oct",
        22 => "BPM",
        23 => "beats",
        24 => "ms",
        _ => "",
    }
}

fn read_params(unit: Unit) -> Vec<Meta> {
    let mut size = 0u32;
    let r = unsafe { AudioUnitGetPropertyInfo(unit, PROP_PARAMETER_LIST, GLOBAL, 0, &mut size, std::ptr::null_mut()) };
    if r != 0 || size == 0 {
        return Vec::new();
    }
    let mut ids = vec![0u32; size as usize / 4];
    let r = unsafe { AudioUnitGetProperty(unit, PROP_PARAMETER_LIST, GLOBAL, 0, ids.as_mut_ptr().cast(), &mut size) };
    if r != 0 {
        return Vec::new();
    }
    ids.truncate(size as usize / 4);
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let mut raw: RawParamInfo = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<RawParamInfo>() as u32;
        if unsafe { AudioUnitGetProperty(unit, PROP_PARAMETER_INFO, GLOBAL, id, (&raw mut raw).cast(), &mut size) } != 0 {
            continue;
        }
        let title = if raw.flags & FLAG_HAS_CF_NAME != 0 && !raw.cf_name.is_null() {
            let s = string_of(raw.cf_name);
            if raw.flags & FLAG_CF_NAME_RELEASE != 0 {
                unsafe { CFRelease(raw.cf_name) };
            }
            s
        } else {
            unsafe { CStr::from_ptr(raw.name.as_ptr()) }.to_string_lossy().into_owned()
        };
        let units = if raw.unit == UNIT_CUSTOM && !raw.unit_name.is_null() {
            let s = string_of(raw.unit_name);
            if raw.flags & FLAG_CF_NAME_RELEASE != 0 {
                unsafe { CFRelease(raw.unit_name) };
            }
            s
        } else {
            unit_label(raw.unit).to_string()
        };
        let mut names = Vec::new();
        if raw.flags & FLAG_HAS_STRINGS != 0 {
            let mut list: Cfref = std::ptr::null();
            let mut size = std::mem::size_of::<Cfref>() as u32;
            let r = unsafe { AudioUnitGetProperty(unit, PROP_VALUE_STRINGS, GLOBAL, id, (&raw mut list).cast(), &mut size) };
            if let (0, Some(list)) = (r, Cf::new(list)) {
                names = (0..unsafe { CFArrayGetCount(list.0) }).map(|i| string_of(unsafe { CFArrayGetValueAtIndex(list.0, i) })).collect();
            }
        }
        let stepped = raw.unit == UNIT_INDEXED || raw.unit == UNIT_BOOLEAN || !names.is_empty();
        let steps = if stepped { (raw.max - raw.min).round().max(0.0) as i32 } else { 0 };
        let default = if raw.max > raw.min { ((raw.default - raw.min) as f64 / (raw.max - raw.min) as f64).clamp(0.0, 1.0) } else { 0.0 };
        out.push(Meta {
            info: ParamInfo { id, short: title.clone(), title, units, steps, default },
            min: raw.min,
            max: raw.max,
            flags: raw.flags | if raw.unit == UNIT_METERS { FLAG_METER } else { 0 },
            names,
            knob: AtomicU64::new(NOT_MODDED),
            played: AtomicU64::new(0),
        });
    }
    out
}

/// Each output element's channels, of `MAX_BLOCK` frames.
type Outputs = Vec<Vec<Vec<f32>>>;

/// A running Audio Unit.
pub struct AudioUnit {
    shared: Arc<Shared>,
    manufacturer: u32,
    kind: u32,
    sub: u32,
    outputs: Outputs,
    /// Takes notes (an instrument, or a MIDI-controlled effect).
    takes_midi: bool,
    has_sidechain: bool,
    sample_rate: f64,
    clock: i64,
    was_playing: bool,
    held: [bool; 128],
    moved: [bool; 130],
    /// Parameters modulation drives (room made with the plug-in).
    modded: Vec<u32>,
    /// The Maschine chunk last applied.
    pub(crate) chunk: Vec<u8>,
    /// That chunk is of a kind we can't load (NI's private ones): it isn't replaced for
    /// differing from what plays, only for a real edit.
    unread_chunk: bool,
}

// As for VST3: used from any thread, one at a time, under the instance's mutex.
unsafe impl Send for AudioUnit {}

impl AudioUnit {
    /// Make an instance of the component the id names (see [`state::au_codes`]).
    pub fn new(id: &crate::PluginId, sample_rate: f64) -> Result<AudioUnit, String> {
        let (kind, sub) = state::au_codes(&id.cid);
        let component = find_component(kind, sub, &id.vendor).ok_or("not installed as an Audio Unit")?;
        let mut desc = Desc::default();
        unsafe { AudioComponentGetDescription(component, &mut desc) };
        let mut unit = std::ptr::null_mut();
        let r = unsafe { AudioComponentInstanceNew(component, &mut unit) };
        if r != 0 || unit.is_null() {
            return Err(format!("creating the Audio Unit failed ({r})"));
        }
        Self::start(unit, desc, sample_rate)
    }

    fn start(unit: Unit, desc: Desc, sample_rate: f64) -> Result<AudioUnit, String> {
        let max = MAX_BLOCK as u32;
        unsafe { AudioUnitSetProperty(unit, PROP_MAX_FRAMES, GLOBAL, 0, (&raw const max).cast(), 4) };
        let context = Box::new(RenderContext {
            input: [vec![0f32; MAX_BLOCK], vec![0f32; MAX_BLOCK]],
            input_channels: 2,
            sidechain: [vec![0f32; MAX_BLOCK], vec![0f32; MAX_BLOCK]],
            sc_channels: 2,
            sample_rate,
            tempo: 120.0,
            quarters: 0.0,
            playing: false,
            changed: false,
            numerator: 4,
            denominator: 4,
        });
        let context = Box::into_raw(context);
        let started = Self::negotiate(unit, desc, sample_rate, context);
        let (input_channels, sc_channels, outputs) = match started {
            Ok(v) => v,
            Err(e) => {
                unsafe {
                    AudioComponentInstanceDispose(unit);
                    drop(Box::from_raw(context));
                }
                return Err(e);
            }
        };
        unsafe {
            (*context).input_channels = input_channels;
            (*context).sc_channels = sc_channels.max(1);
        }
        let params = read_params(unit);
        let index = params.iter().enumerate().map(|(i, m)| (m.info.id, i)).collect();
        let shared = Arc::new(Shared {
            unit,
            context,
            params,
            index,
            dirty: AtomicBool::new(false),
            window_seen: AtomicBool::new(false),
            listener: AtomicPtr::new(std::ptr::null_mut()),
        });
        listen(&shared);
        let midi = [u32::from_be_bytes(*b"aumu"), u32::from_be_bytes(*b"aumf")];
        Ok(AudioUnit {
            shared,
            manufacturer: desc.manufacturer,
            kind: desc.kind,
            sub: desc.sub,
            outputs,
            takes_midi: midi.contains(&desc.kind),
            has_sidechain: sc_channels > 0,
            sample_rate,
            clock: 0,
            was_playing: false,
            held: [false; 128],
            moved: [false; 130],
            modded: Vec::with_capacity(MODDED),
            chunk: Vec::new(),
            unread_chunk: false,
        })
    }

    /// Settle the stream formats (stereo if it'll take it), and initialize. Returns the
    /// main input's channels, the side-chain input's channels (0 if none), and buffers
    /// for each output element.
    fn negotiate(unit: Unit, desc: Desc, rate: f64, context: *mut RenderContext) -> Result<(usize, usize, Outputs), String> {
        let is_effect = desc.kind != u32::from_be_bytes(*b"aumu");
        let inputs = get_u32(unit, PROP_ELEMENT_COUNT, INPUT, 0).unwrap_or(0);
        let elements = get_u32(unit, PROP_ELEMENT_COUNT, OUTPUT, 0).unwrap_or(1).max(1);
        let takes_input = is_effect && inputs > 0;
        let has_sc = takes_input && inputs > 1;
        if takes_input {
            let callback = RenderCallback { proc_: give_input, refcon: context.cast() };
            let r = unsafe {
                AudioUnitSetProperty(
                    unit,
                    PROP_RENDER_CALLBACK,
                    INPUT,
                    0,
                    (&raw const callback).cast(),
                    std::mem::size_of::<RenderCallback>() as u32,
                )
            };
            if r != 0 {
                return Err(format!("connecting the Audio Unit's input failed ({r})"));
            }
            if has_sc {
                unsafe {
                    AudioUnitSetProperty(
                        unit,
                        PROP_RENDER_CALLBACK,
                        INPUT,
                        1,
                        (&raw const callback).cast(),
                        std::mem::size_of::<RenderCallback>() as u32,
                    )
                };
            }
        }
        let callbacks =
            HostCallbacks { user: context.cast(), beat_and_tempo, musical_time, transport: transport_state, transport2: transport_state2 };
        // (Not every plug-in has these: one that doesn't has no use for them.)
        unsafe {
            AudioUnitSetProperty(
                unit,
                PROP_HOST_CALLBACKS,
                GLOBAL,
                0,
                (&raw const callbacks).cast(),
                std::mem::size_of::<HostCallbacks>() as u32,
            )
        };
        let tries: &[(u32, u32)] = if takes_input { &[(2, 2), (1, 2), (1, 1), (2, 1)] } else { &[(0, 2), (0, 1)] };
        let mut last = 0;
        let mut started = false;
        for &(i, o) in tries {
            if i > 0 && set_format(unit, INPUT, 0, &stream_format(rate, i)) != 0 {
                continue;
            }
            if has_sc && set_format(unit, INPUT, 1, &stream_format(rate, 2)) != 0 {
                let _ = set_format(unit, INPUT, 1, &stream_format(rate, 1));
            }
            last = set_format(unit, OUTPUT, 0, &stream_format(rate, o));
            if last != 0 {
                continue;
            }
            // Other outputs stereo if they'll have it; they keep theirs if not.
            for e in 1..elements {
                set_format(unit, OUTPUT, e, &stream_format(rate, 2));
            }
            last = unsafe { AudioUnitInitialize(unit) };
            if last == 0 {
                started = true;
                break;
            }
        }
        if !started {
            return Err(format!("no stereo or mono layout works for this Audio Unit ({last})"));
        }
        let input_channels = if takes_input { channels_of(unit, INPUT, 0).unwrap_or(2).clamp(1, 2) } else { 2 };
        let sc_channels = if has_sc { channels_of(unit, INPUT, 1).unwrap_or(2).clamp(1, 2) } else { 0 };
        let outputs = (0..elements)
            .map(|e| {
                // An element in a layout this isn't set up for (more than two channels) isn't used.
                let n = channels_of(unit, OUTPUT, e).unwrap_or(0);
                if (1..=2).contains(&n) { vec![vec![0f32; MAX_BLOCK]; n] } else { Vec::new() }
            })
            .collect();
        Ok((input_channels, sc_channels, outputs))
    }

    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// `component` is the plug-in's ClassInfo as a property list (see [`AudioUnit::state`]).
    pub fn set_state(&mut self, state: &State) -> Result<(), String> {
        if state.component.is_empty() {
            return Ok(());
        }
        let dict = dict_from_plist(&state.component).ok_or("the plug-in's state doesn't read")?;
        self.apply(&dict)
    }

    fn apply(&mut self, dict: &Cf) -> Result<(), String> {
        let unit = self.shared.unit;
        let value: Cfref = dict.0;
        let r = unsafe {
            AudioUnitSetProperty(unit, PROP_CLASS_INFO, GLOBAL, 0, (&raw const value).cast(), std::mem::size_of::<Cfref>() as u32)
        };
        unsafe { AudioUnitReset(unit, GLOBAL, 0) };
        self.held = [false; 128];
        self.moved = [false; 130];
        // What modulation drives: the state just loaded is its knob now, and it goes on
        // playing modulation's value.
        for &id in &self.modded {
            if let Some(m) = self.shared.meta(id) {
                m.knob.store(m.normalized(self.shared.played_value(id)).to_bits(), Ordering::Release);
                let played = f64::from_bits(m.played.load(Ordering::Acquire));
                unsafe { AudioUnitSetParameter(unit, id, GLOBAL, 0, m.plain(played), 0) };
            }
        }
        self.shared.notify_all();
        if r == 0 { Ok(()) } else { Err(format!("restoring the plug-in's state failed ({r})")) }
    }

    /// The state Maschine saved: types 1 and 4 are a plug-in's raw data, type 3 the whole
    /// of its ClassInfo. Others are NI's private ones: the plug-in stays at its defaults.
    pub fn load_chunk(&mut self, chunk: &[u8]) -> Result<(), String> {
        let (kind, data) = state::chunk_parts(chunk)?;
        self.unread_chunk = !data.is_empty() && !matches!(kind, CHUNK_PLIST | CHUNK_RAW | CHUNK_RAW_ALSO);
        if data.is_empty() {
            return Ok(());
        }
        match kind {
            CHUNK_PLIST => {
                let dict = dict_from_plist(data).ok_or("the plug-in's state doesn't read")?;
                self.apply(&dict)
            }
            CHUNK_RAW | CHUNK_RAW_ALSO => {
                let dict = self.dict_from_raw(data);
                self.apply(&dict)
            }
            _ => Ok(()),
        }
    }

    fn dict_from_raw(&self, data: &[u8]) -> Cf {
        let keys = (&raw const kCFTypeDictionaryKeyCallBacks, &raw const kCFTypeDictionaryValueCallBacks);
        let dict = unsafe { CFDictionaryCreateMutable(std::ptr::null(), 0, keys.0, keys.1) };
        let set = |k: &str, v: Cf| unsafe { CFDictionarySetValue(dict, cf_string(k).0, v.0) };
        set("version", cf_i32(0));
        set("type", cf_i32(self.kind as i32));
        set("subtype", cf_i32(self.sub as i32));
        set("manufacturer", cf_i32(self.manufacturer as i32));
        set("vstdata", Cf(unsafe { CFDataCreate(std::ptr::null(), data.as_ptr(), data.len() as isize) }));
        Cf(dict)
    }

    /// The plug-in's ClassInfo, with each parameter modulation drives at its knob's value.
    fn read_class_info(&self) -> Option<Cf> {
        let unit = self.shared.unit;
        for &id in &self.modded {
            if let Some(m) = self.shared.meta(id)
                && let Some(k) = m.knob()
            {
                unsafe { AudioUnitSetParameter(unit, id, GLOBAL, 0, m.plain(k), 0) };
            }
        }
        let mut value: Cfref = std::ptr::null();
        let mut size = std::mem::size_of::<Cfref>() as u32;
        let r = unsafe { AudioUnitGetProperty(unit, PROP_CLASS_INFO, GLOBAL, 0, (&raw mut value).cast(), &mut size) };
        for &id in &self.modded {
            if let Some(m) = self.shared.meta(id) {
                let played = f64::from_bits(m.played.load(Ordering::Acquire));
                unsafe { AudioUnitSetParameter(unit, id, GLOBAL, 0, m.plain(played), 0) };
            }
        }
        if r == 0 { Cf::new(value) } else { None }
    }

    /// The plug-in's current state: its ClassInfo as an XML property list.
    pub fn state(&mut self) -> State {
        let component = self.read_class_info().and_then(|d| xml_of(&d)).unwrap_or_default();
        State { component, controller: Vec::new() }
    }

    /// The state as Maschine saves it. If the plug-in won't give one, what was loaded:
    /// never an empty one in place of its settings.
    pub fn save_chunk(&self) -> Vec<u8> {
        let Some(dict) = self.read_class_info() else { return self.chunk.clone() };
        if self.manufacturer == NI {
            let key = cf_string("vstdata");
            let data = unsafe { CFDictionaryGetValue(dict.0, key.0) };
            if !data.is_null() {
                return state::wrap_typed(CHUNK_RAW, &bytes_of_data(data));
            }
        }
        xml_of(&dict).map_or_else(|| self.chunk.clone(), |xml| state::wrap_typed(CHUNK_PLIST, &xml))
    }

    pub fn parameter_list(&self, automatable: bool) -> Vec<ParamInfo> {
        let wanted = |m: &&Meta| {
            !automatable || (m.flags & FLAG_READABLE != 0 && m.flags & FLAG_WRITABLE != 0 && m.flags & FLAG_METER == 0 && m.max > m.min)
        };
        self.shared.params.iter().filter(wanted).map(|m| m.info.clone()).collect()
    }

    pub fn param_value(&self, id: u32) -> f64 {
        self.shared.value(id)
    }

    pub fn param_text(&self, id: u32, v: f64) -> String {
        self.shared.text(id, v)
    }

    pub fn set_param(&mut self, id: u32, v: f64) {
        self.shared.set(id, v);
    }

    /// Something was edited: reported by the plug-in, or (its window having been open)
    /// its state isn't what the project has.
    pub fn take_dirty(&self) -> bool {
        self.shared.dirty.swap(false, Ordering::AcqRel)
            || (self.shared.window_seen.load(Ordering::Acquire) && !self.unread_chunk && self.save_chunk() != self.chunk)
    }

    pub fn modulate(&mut self, id: u32, v: f64) {
        let v = v.clamp(0.0, 1.0);
        let Some(m) = self.shared.meta(id) else { return };
        if m.knob().is_none() {
            if self.modded.len() >= MODDED {
                return;
            }
            m.knob.store(m.normalized(self.shared.played_value(id)).to_bits(), Ordering::Release);
            self.modded.push(id);
        }
        m.played.store(v.to_bits(), Ordering::Release);
        unsafe { AudioUnitSetParameter(self.shared.unit, id, GLOBAL, 0, m.plain(v), 0) };
    }

    pub fn unmodulate(&mut self, id: u32) {
        let Some(i) = self.modded.iter().position(|&m| m == id) else { return };
        self.modded.remove(i);
        let Some(m) = self.shared.meta(id) else { return };
        let knob = m.knob.swap(NOT_MODDED, Ordering::AcqRel);
        if knob != NOT_MODDED {
            unsafe { AudioUnitSetParameter(self.shared.unit, id, GLOBAL, 0, m.plain(f64::from_bits(knob)), 0) };
        }
    }

    pub fn modulated(&self, id: u32) -> Option<f64> {
        let m = self.shared.meta(id)?;
        m.knob().map(|_| f64::from_bits(m.played.load(Ordering::Acquire)))
    }

    fn midi(&self, status: u32, d1: u32, d2: u32) {
        if self.takes_midi {
            unsafe { MusicDeviceMIDIEvent(self.shared.unit, status, d1, d2, 0) };
        }
    }

    pub fn note_on(&mut self, pitch: i64, velocity: i64) {
        let pitch = pitch.clamp(0, 127);
        if self.held[pitch as usize] {
            self.note_off(pitch);
        }
        self.held[pitch as usize] = true;
        self.midi(0x90, pitch as u32, velocity.clamp(1, 127) as u32);
    }

    pub fn note_off(&mut self, pitch: i64) {
        let pitch = pitch.clamp(0, 127);
        if std::mem::take(&mut self.held[pitch as usize]) {
            self.midi(0x80, pitch as u32, 0);
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

    /// AUs take MIDI as MIDI: every controller, if it takes MIDI at all.
    pub fn takes_controller(&self, number: usize) -> bool {
        self.takes_midi && (number <= 127 || number == 129)
    }

    pub fn control_change(&mut self, number: u8, value: u8) {
        let (n, v) = (number.min(127), value.min(127));
        self.moved[n as usize] = v != 0;
        self.midi(0xB0, n as u32, v as u32);
    }

    pub fn pitch_bend(&mut self, value: u16) {
        let v = value.min(16383);
        self.moved[129] = v != 8192;
        self.midi(0xE0, (v & 0x7F) as u32, (v >> 7) as u32);
    }

    pub fn has_sidechain(&self) -> bool {
        self.has_sidechain
    }

    pub fn process_input_sc(
        &mut self,
        frames: usize,
        transport: &Transport,
        input: Option<(&[f32], &[f32])>,
        sidechain: Option<(&[f32], &[f32])>,
    ) -> (&[f32], &[f32]) {
        let frames = frames.min(MAX_BLOCK);
        if frames == 0 || self.outputs.first().is_none_or(|e| e.is_empty()) {
            return (&[], &[]);
        }
        {
            let ctx = unsafe { &mut *self.shared.context };
            for (dst, ch, src) in [(&mut ctx.input, ctx.input_channels, input), (&mut ctx.sidechain, ctx.sc_channels, sidechain)] {
                dst[0][..frames].fill(0.0);
                dst[1][..frames].fill(0.0);
                if let Some((l, r)) = src {
                    let n = frames.min(l.len()).min(r.len());
                    if ch == 1 {
                        for (o, (a, b)) in dst[0][..n].iter_mut().zip(l.iter().zip(r)) {
                            *o = (a + b) * 0.5;
                        }
                    } else {
                        dst[0][..n].copy_from_slice(&l[..n]);
                        dst[1][..n].copy_from_slice(&r[..n]);
                    }
                }
            }
            ctx.tempo = if transport.tempo > 0.0 { transport.tempo } else { 120.0 };
            ctx.quarters = transport.quarters;
            ctx.changed = transport.playing != self.was_playing;
            ctx.playing = transport.playing;
            ctx.numerator = transport.numerator.max(1);
            ctx.denominator = transport.denominator.max(1);
            self.was_playing = transport.playing;
        }
        const SAMPLE_TIME_VALID: u32 = 1;
        let ts = TimeStamp { sample_time: self.clock as f64, flags: SAMPLE_TIME_VALID, ..TimeStamp::default() };
        let unit = self.shared.unit;
        for (bus, element) in self.outputs.iter_mut().enumerate() {
            if element.is_empty() {
                continue;
            }
            let empty = Buffer { channels: 1, bytes: 0, data: std::ptr::null_mut() };
            let mut list = BufferList { count: element.len() as u32, buffers: [empty; 2] };
            for (b, c) in list.buffers.iter_mut().zip(element.iter_mut()) {
                *b = Buffer { channels: 1, bytes: (frames * 4) as u32, data: c.as_mut_ptr().cast() };
            }
            let mut flags = 0u32;
            let r = unsafe { AudioUnitRender(unit, &mut flags, &ts, bus as u32, frames as u32, &mut list) };
            for (b, c) in list.buffers.iter().zip(element.iter_mut()) {
                if r != 0 || flags & RENDER_OUTPUT_IS_SILENCE != 0 || b.data.is_null() {
                    c[..frames].fill(0.0);
                } else if b.data != c.as_mut_ptr().cast() {
                    // The plug-in gave its own buffer.
                    let theirs = unsafe { std::slice::from_raw_parts(b.data as *const f32, frames) };
                    c[..frames].copy_from_slice(theirs);
                }
            }
        }
        self.clock += frames as i64;
        let main = &self.outputs[0];
        (&main[0][..frames], &main[main.len() - 1][..frames])
    }

    pub fn output_bus(&self, bus: usize, frames: usize) -> Option<(&[f32], &[f32])> {
        let b = self.outputs.get(bus).filter(|b| !b.is_empty())?;
        let n = frames.min(MAX_BLOCK);
        Some((&b[0][..n], &b[b.len() - 1][..n]))
    }
}

/// Hear about edits made in the plug-in (its window), on the main run loop.
fn listen(shared: &Arc<Shared>) {
    let user = Arc::as_ptr(shared) as *mut c_void;
    let mut listener = std::ptr::null_mut();
    let r = unsafe { AUEventListenerCreate(edited, user, CFRunLoopGetMain(), kCFRunLoopCommonModes, 0.1, 0.1, &mut listener) };
    if r != 0 || listener.is_null() {
        return;
    }
    let param = shared.target(ANY_PARAMETER);
    let events = [
        AuEvent { kind: EVENT_PARAMETER_CHANGED, target: param },
        AuEvent { kind: EVENT_GESTURE_END, target: param },
        AuEvent { kind: EVENT_PROPERTY_CHANGED, target: shared.target(PROP_CLASS_INFO) },
        AuEvent { kind: EVENT_PROPERTY_CHANGED, target: shared.target(PROP_PRESENT_PRESET) },
    ];
    for e in &events {
        unsafe { AUEventListenerAddEventType(listener, std::ptr::null_mut(), e) };
    }
    shared.listener.store(listener, Ordering::Release);
}

fn dict_from_plist(data: &[u8]) -> Option<Cf> {
    let d = Cf::new(unsafe { CFDataCreate(std::ptr::null(), data.as_ptr(), data.len() as isize) })?;
    Cf::new(unsafe { CFPropertyListCreateWithData(std::ptr::null(), d.0, 0, std::ptr::null_mut(), std::ptr::null_mut()) })
}

fn xml_of(dict: &Cf) -> Option<Vec<u8>> {
    let d = Cf::new(unsafe { CFPropertyListCreateData(std::ptr::null(), dict.0, XML_PLIST, 0, std::ptr::null_mut()) })?;
    Some(bytes_of_data(d.0))
}

/// `[object selector:…]` with C types, where `msg_send!` would check them against the
/// method's own (an AudioUnit is a pointer to a type of Apple's).
unsafe fn send_view_for(factory: *mut AnyObject, unit: Unit, size: NSSize) -> *mut AnyObject {
    type Send = unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, Unit, NSSize) -> *mut AnyObject;
    let send: Send = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { send(factory, objc2::sel!(uiViewForAudioUnit:withSize:), unit, size) }
}

/// `setFrameOrigin:` the same way: NI's views declare it unusually.
fn set_origin_to_zero(view: &NSView) {
    type Send = unsafe extern "C" fn(*const NSView, objc2::runtime::Sel, NSPoint);
    let send: Send = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { send(view, objc2::sel!(setFrameOrigin:), NSPoint::new(0.0, 0.0)) }
}

unsafe fn send_generic_view(view: *mut AnyObject, unit: Unit) -> *mut AnyObject {
    type Send = unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, Unit) -> *mut AnyObject;
    let send: Send = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { send(view, objc2::sel!(initWithAudioUnit:), unit) }
}

/// The plug-in's own view, and the object that made it.
fn own_view(unit: Unit) -> Option<(Retained<NSView>, Retained<AnyObject>)> {
    use objc2::{class, msg_send};
    let mut size = 0u32;
    if unsafe { AudioUnitGetPropertyInfo(unit, PROP_COCOA_UI, GLOBAL, 0, &mut size, std::ptr::null_mut()) } != 0
        || (size as usize) < 2 * std::mem::size_of::<Cfref>()
    {
        return None;
    }
    // AudioUnitCocoaViewInfo: the bundle's URL, then class names.
    let mut info = vec![std::ptr::null::<c_void>(); size as usize / std::mem::size_of::<Cfref>()];
    if unsafe { AudioUnitGetProperty(unit, PROP_COCOA_UI, GLOBAL, 0, info.as_mut_ptr().cast(), &mut size) } != 0 {
        return None;
    }
    let url = Cf::new(info[0])?;
    let class_name = Cf::new(info[1])?;
    unsafe {
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), bundleWithURL: url.0 as *mut AnyObject];
        if bundle.is_null() {
            return None;
        }
        let class: *mut AnyObject = msg_send![bundle, classNamed: class_name.0 as *mut AnyObject];
        if class.is_null() {
            return None;
        }
        let factory: *mut AnyObject = msg_send![class, new];
        let factory = Retained::from_raw(factory)?;
        let view = send_view_for(Retained::as_ptr(&factory) as *mut AnyObject, unit, NSSize::new(640.0, 480.0));
        Some((Retained::retain(view as *mut NSView)?, factory))
    }
}

/// CoreAudioKit's generic view, for a plug-in that has none of its own. The framework is
/// loaded here, so that the app only needs it when one of these opens.
fn generic_view(unit: Unit) -> Option<Retained<NSView>> {
    use objc2::{class, msg_send};
    unsafe {
        let path = NSString::from_str("/System/Library/Frameworks/CoreAudioKit.framework");
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), bundleWithPath: &*path];
        if bundle.is_null() {
            return None;
        }
        let loaded: bool = msg_send![bundle, load];
        if !loaded {
            return None;
        }
        let class = objc2::runtime::AnyClass::get(c"AUGenericView")?;
        let view: *mut AnyObject = msg_send![class, alloc];
        Retained::from_raw(send_generic_view(view, unit) as *mut NSView)
    }
}

/// An open Audio Unit window.
pub struct Editor {
    // (Dropped in this order: the view goes before the plug-in it shows.)
    window: Retained<NSWindow>,
    view: Retained<NSView>,
    /// Keeps what made the view as long as the view.
    _factory: Option<Retained<AnyObject>>,
    size: Cell<(f64, f64)>,
    plugin: crate::Shared,
}

impl Editor {
    pub fn open(plugin: &crate::Shared, shared: Arc<Shared>, title: &str) -> Result<Editor, String> {
        let mtm = MainThreadMarker::new().ok_or("plug-in windows can only be opened on the main thread")?;
        let (view, factory) = match own_view(shared.unit) {
            Some((v, f)) => (v, Some(f)),
            None => (generic_view(shared.unit).ok_or("the plug-in has no editor")?, None),
        };
        let size = view.frame().size;
        let (w, h) = (size.width.max(100.0), size.height.max(60.0));
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h)),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        window.center();
        let content = window.contentView().ok_or("the window has no content view")?;
        set_origin_to_zero(&view);
        content.addSubview(&view);
        window.makeKeyAndOrderFront(None);
        shared.window_seen.store(true, Ordering::Release);
        Ok(Editor { window, view, _factory: factory, size: Cell::new((w, h)), plugin: plugin.clone() })
    }

    /// The plug-in resized its view: so does the window.
    pub fn tick(&self) {
        let s = self.view.frame().size;
        let (w, h) = (s.width.max(100.0), s.height.max(60.0));
        if self.size.get() != (w, h) {
            self.size.set((w, h));
            self.window.setContentSize(NSSize::new(w, h));
            set_origin_to_zero(&self.view);
        }
    }

    pub fn is_open(&self) -> bool {
        self.window.isVisible()
    }

    pub fn raise(&self) {
        self.window.makeKeyAndOrderFront(None);
    }

    pub fn plugin(&self) -> &crate::Shared {
        &self.plugin
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.view.removeFromSuperview();
        self.window.close();
    }
}
