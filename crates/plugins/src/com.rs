//! The host side of VST3: the objects a host hands to plug-ins (host context, messages,
//! streams, event lists, parameter changes, component handler).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Arc, Mutex};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComWrapper, Interface};

pub const HOST_NAME: &str = "DECKARD";

fn guid(t: *const TUID) -> [u8; 16] {
    unsafe { std::ptr::read(t as *const [u8; 16]) }
}

/// Write `s` into a VST3 UTF-16 string buffer.
pub fn write_string128(s: &str, dst: &mut String128) {
    let mut n = 0;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16()).take(127) {
        *d = c;
        n += 1;
    }
    dst[n] = 0;
}

/// A VST3 UTF-16 string.
pub fn read_string128(s: &[TChar]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

/// The host context passed to `initialize`: names the host and makes messages.
pub struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        unsafe { write_string128(HOST_NAME, &mut *name) };
        kResultOk
    }

    unsafe fn createInstance(&self, cid: *mut TUID, iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        let (cid, iid) = (guid(cid), guid(iid));
        unsafe {
            if cid == IMessage::IID && (iid == IMessage::IID || iid == FUnknown::IID) {
                *obj = ComWrapper::new(Message::default()).to_com_ptr::<IMessage>().unwrap().into_raw() as *mut c_void;
                return kResultOk;
            }
            if cid == IAttributeList::IID && (iid == IAttributeList::IID || iid == FUnknown::IID) {
                *obj = ComWrapper::new(AttributeList::default()).to_com_ptr::<IAttributeList>().unwrap().into_raw() as *mut c_void;
                return kResultOk;
            }
            *obj = std::ptr::null_mut();
        }
        kResultFalse
    }
}

/// A message between a plug-in's processor and controller (they talk through the host).
pub struct Message {
    id: RefCell<CString>,
    attributes: ComWrapper<AttributeList>,
}

impl Default for Message {
    fn default() -> Message {
        Message { id: RefCell::new(CString::default()), attributes: ComWrapper::new(AttributeList::default()) }
    }
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id.borrow().as_ptr()
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        if !id.is_null() {
            *self.id.borrow_mut() = unsafe { CStr::from_ptr(id) }.to_owned();
        }
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        // Borrowed, as in the SDK's host implementation.
        self.attributes.as_com_ref::<IAttributeList>().unwrap().as_ptr()
    }
}

enum Attr {
    Int(i64),
    Float(f64),
    /// UTF-16 with its terminating 0.
    Str(Vec<TChar>),
    Bin(Vec<u8>),
}

#[derive(Default)]
pub struct AttributeList {
    values: RefCell<HashMap<Vec<u8>, Attr>>,
}

fn key(id: *const c_char) -> Option<Vec<u8>> {
    (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_bytes().to_vec())
}

impl Class for AttributeList {
    type Interfaces = (IAttributeList,);
}

impl IAttributeListTrait for AttributeList {
    unsafe fn setInt(&self, id: IAttributeList_::AttrID, value: int64) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        self.values.borrow_mut().insert(k, Attr::Int(value));
        kResultOk
    }

    unsafe fn getInt(&self, id: IAttributeList_::AttrID, value: *mut int64) -> tresult {
        match key(id).and_then(|k| match self.values.borrow().get(&k) {
            Some(Attr::Int(v)) => Some(*v),
            _ => None,
        }) {
            Some(v) => {
                unsafe { *value = v };
                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: IAttributeList_::AttrID, value: f64) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        self.values.borrow_mut().insert(k, Attr::Float(value));
        kResultOk
    }

    unsafe fn getFloat(&self, id: IAttributeList_::AttrID, value: *mut f64) -> tresult {
        match key(id).and_then(|k| match self.values.borrow().get(&k) {
            Some(Attr::Float(v)) => Some(*v),
            _ => None,
        }) {
            Some(v) => {
                unsafe { *value = v };
                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: IAttributeList_::AttrID, string: *const TChar) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut s = Vec::new();
        let mut i = 0;
        loop {
            let c = unsafe { *string.add(i) };
            s.push(c);
            if c == 0 {
                break;
            }
            i += 1;
        }
        self.values.borrow_mut().insert(k, Attr::Str(s));
        kResultOk
    }

    unsafe fn getString(&self, id: IAttributeList_::AttrID, string: *mut TChar, size_in_bytes: uint32) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        let values = self.values.borrow();
        let Some(Attr::Str(s)) = values.get(&k) else { return kResultFalse };
        let room = size_in_bytes as usize / 2;
        if room == 0 {
            return kResultFalse;
        }
        let n = s.len().min(room);
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), string, n);
            *string.add(n - 1) = 0;
        }
        kResultOk
    }

    unsafe fn setBinary(&self, id: IAttributeList_::AttrID, data: *const c_void, size_in_bytes: uint32) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        let bytes = if data.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize) }.to_vec()
        };
        self.values.borrow_mut().insert(k, Attr::Bin(bytes));
        kResultOk
    }

    unsafe fn getBinary(&self, id: IAttributeList_::AttrID, data: *mut *const c_void, size_in_bytes: *mut uint32) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        let values = self.values.borrow();
        let Some(Attr::Bin(b)) = values.get(&k) else { return kResultFalse };
        // The bytes stay where they are until the attribute is set again.
        unsafe {
            *data = b.as_ptr() as *const c_void;
            *size_in_bytes = b.len() as uint32;
        }
        kResultOk
    }
}

/// An in-memory `IBStream` (plug-in state in and out).
#[derive(Default)]
pub struct Stream {
    data: RefCell<Vec<u8>>,
    pos: Cell<usize>,
}

impl Stream {
    pub fn new(data: Vec<u8>) -> ComWrapper<Stream> {
        ComWrapper::new(Stream { data: RefCell::new(data), pos: Cell::new(0) })
    }

    pub fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.data.borrow_mut())
    }
}

impl Class for Stream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, num_bytes_read: *mut int32) -> tresult {
        let data = self.data.borrow();
        let pos = self.pos.get().min(data.len());
        let n = (num_bytes.max(0) as usize).min(data.len() - pos);
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr().add(pos), buffer as *mut u8, n);
            if !num_bytes_read.is_null() {
                *num_bytes_read = n as int32;
            }
        }
        self.pos.set(pos + n);
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, num_bytes: int32, num_bytes_written: *mut int32) -> tresult {
        let n = num_bytes.max(0) as usize;
        let mut data = self.data.borrow_mut();
        let pos = self.pos.get();
        if data.len() < pos + n {
            data.resize(pos + n, 0);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(buffer as *const u8, data.as_mut_ptr().add(pos), n);
            if !num_bytes_written.is_null() {
                *num_bytes_written = n as int32;
            }
        }
        self.pos.set(pos + n);
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let len = self.data.borrow().len() as i64;
        // The seek modes are C enum constants: u32 on macOS, i32 on Windows.
        use IBStream_::IStreamSeekMode_::{kIBSeekCur, kIBSeekEnd, kIBSeekSet};
        let base = match mode as i64 {
            m if m == kIBSeekSet as i64 => 0,
            m if m == kIBSeekCur as i64 => self.pos.get() as i64,
            m if m == kIBSeekEnd as i64 => len,
            _ => return kInvalidArgument,
        };
        let to = (base + pos).max(0);
        self.pos.set(to as usize);
        if !result.is_null() {
            unsafe { *result = to };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if !pos.is_null() {
            unsafe { *pos = self.pos.get() as int64 };
        }
        kResultOk
    }
}

/// Note events for one process call.
#[derive(Default)]
pub struct EventList {
    pub events: RefCell<Vec<Event>>,
}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        self.events.borrow().len() as int32
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        match self.events.borrow().get(index as usize) {
            Some(ev) => {
                unsafe { *e = *ev };
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        self.events.borrow_mut().push(unsafe { *e });
        kResultOk
    }
}

/// Changes of one parameter within a block.
pub struct ParamQueue {
    id: Cell<ParamID>,
    points: RefCell<Vec<(int32, ParamValue)>>,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points.borrow().len() as int32
    }

    unsafe fn getPoint(&self, index: int32, sample_offset: *mut int32, value: *mut ParamValue) -> tresult {
        match self.points.borrow().get(index as usize) {
            Some(&(o, v)) => {
                unsafe {
                    *sample_offset = o;
                    *value = v;
                }
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn addPoint(&self, sample_offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let mut p = self.points.borrow_mut();
        p.push((sample_offset, value));
        if !index.is_null() {
            unsafe { *index = p.len() as int32 - 1 };
        }
        kResultOk
    }
}

/// Parameter changes for one process call: the first `len` queues. Cleared queues stay,
/// with their room, for the next call's changes (the audio thread allocates only while
/// they grow).
#[derive(Default)]
pub struct ParameterChanges {
    queues: RefCell<Vec<ComWrapper<ParamQueue>>>,
    len: Cell<usize>,
}

impl ParameterChanges {
    pub fn clear(&self) {
        for q in self.queues.borrow().iter().take(self.len.get()) {
            q.points.borrow_mut().clear();
        }
        self.len.set(0);
    }

    /// The index of `id`'s queue in use, taking a cleared one (or making one) for it.
    fn queue(&self, id: ParamID) -> usize {
        let n = self.len.get();
        let mut q = self.queues.borrow_mut();
        if let Some(i) = q.iter().take(n).position(|x| x.id.get() == id) {
            return i;
        }
        if n == q.len() {
            q.push(ComWrapper::new(ParamQueue { id: Cell::new(id), points: RefCell::new(Vec::with_capacity(4)) }));
        }
        q[n].id.set(id);
        self.len.set(n + 1);
        n
    }

    pub fn add(&self, id: ParamID, value: ParamValue) {
        let i = self.queue(id);
        self.queues.borrow()[i].points.borrow_mut().push((0, value));
    }

    /// The last value of each changed parameter.
    pub fn last_values(&self) -> Vec<(ParamID, ParamValue)> {
        self.queues.borrow().iter().take(self.len.get()).filter_map(|q| q.points.borrow().last().map(|&(_, v)| (q.id.get(), v))).collect()
    }
}

impl Class for ParameterChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParameterChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.len.get() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if index < 0 || index as usize >= self.len.get() {
            return std::ptr::null_mut();
        }
        self.queues.borrow().get(index as usize).map_or(std::ptr::null_mut(), |q| q.as_com_ref::<IParamValueQueue>().unwrap().as_ptr())
    }

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        let id = unsafe { *id };
        let i = self.queue(id);
        if !index.is_null() {
            unsafe { *index = i as int32 };
        }
        self.queues.borrow()[i].as_com_ref::<IParamValueQueue>().unwrap().as_ptr()
    }
}

/// Edits made in a plug-in's own editor, for the processor and for saving.
#[derive(Default)]
pub struct Edits {
    /// Parameter changes not yet sent to the processor.
    pub pending: Vec<(ParamID, ParamValue)>,
    /// Something changed since the state was last saved.
    pub dirty: bool,
}

pub struct ComponentHandler {
    pub edits: Arc<Mutex<Edits>>,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        if let Ok(mut e) = self.edits.lock() {
            e.pending.push((id, value));
            e.dirty = true;
        }
        kResultOk
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        if let Ok(mut e) = self.edits.lock() {
            e.dirty = true;
        }
        kResultOk
    }
}

#[cfg(test)]
mod param_tests {
    use super::*;

    /// A block's parameter changes reuse the last block's queues (no allocating on the
    /// audio thread once they've grown).
    #[test]
    fn changes_reuse_their_queues() {
        let c = ParameterChanges::default();
        c.add(1, 0.5);
        c.add(1, 0.6);
        c.clear();
        c.add(2, 0.25);
        assert_eq!(unsafe { c.getParameterCount() }, 1);
        assert_eq!(c.last_values(), vec![(2, 0.25)]);
        assert_eq!(c.queues.borrow().len(), 1, "the queue was reused");
        assert!(c.queues.borrow()[0].points.borrow().capacity() >= 2, "with its room");
        c.clear();
        assert_eq!(unsafe { c.getParameterCount() }, 0);
        assert!(c.last_values().is_empty());
    }
}
