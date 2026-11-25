//! Files dragged out of the window onto another application (a sampler plug-in, a
//! track, the Finder): the system's own drag, begun from the mouse press egui saw. The
//! windowing layer has no such thing, so the system is asked (macOS only for now).

use std::path::PathBuf;

/// Begin dragging `files` (each there on disk) from where the pointer is. The system
/// takes the mouse from here: the window never hears the button come up.
pub fn start(files: &[PathBuf]) -> Result<(), String> {
    if files.is_empty() {
        return Err("nothing to drag".into());
    }
    if let Some(f) = files.iter().find(|f| !f.is_file()) {
        return Err(format!("{} isn't there", f.display()));
    }
    system(files)
}

impl crate::App {
    /// The file a drag out of the Sampling view gives: the focused zone's sample, or its
    /// stem lane `lane`. The part selected, else the part the zone plays, written as a
    /// WAV in the project's `Samples/Dragged` (once: the same part again is the same
    /// file); `whole`, or a part that's all of it: the file itself.
    pub(crate) fn drag_file(&mut self, lane: Option<usize>, whole: bool) -> Result<PathBuf, String> {
        let st = &self.sampling;
        let (sample, file) = match lane {
            None => (st.sample.clone(), st.sample_key.as_ref().map(|k| k.3.clone())),
            Some(k) => {
                let l = st.lanes.get(k).and_then(Option::as_ref);
                (l.map(|l| l.sample.clone()), l.map(|l| l.file.clone()))
            }
        };
        let (Some(sample), Some(file)) = (sample, file) else { return Err("no sample here".into()) };
        let len = sample.frames.len();
        let (a, b) = if whole { (0, len) } else { self.edit_range().unwrap_or((0, len)) };
        let (a, b) = (a.min(len), b.min(len));
        if b <= a || sample.rate == 0 {
            return Err("nothing there".into());
        }
        let original = PathBuf::from(&file);
        let all = (a, b) == (0, len);
        if all && original.is_file() {
            return Ok(original);
        }
        let dir = self.sample_dir("Samples").join("Dragged");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let stem = original.file_stem().map_or("Sample".to_string(), |s| s.to_string_lossy().into_owned());
        let seconds = |f: usize| f as f64 / f64::from(sample.rate);
        let name = if all { format!("{stem}.wav") } else { format!("{stem} {:.3}-{:.3}s.wav", seconds(a), seconds(b)) };
        let target = dir.join(name);
        if !target.is_file() {
            let part = deckard_engine::sample::Sample { rate: sample.rate, frames: sample.frames[a..b].to_vec() };
            deckard_engine::edit::write_wav(&target, &part)?;
        }
        Ok(target)
    }

    /// Drag it out of the window ([`App::drag_file`]).
    pub(crate) fn drag_out(&mut self, lane: Option<usize>, whole: bool) {
        match self.drag_file(lane, whole).and_then(|f| start(std::slice::from_ref(&f)).map(|()| f)) {
            Ok(f) => {
                self.status = format!("Dragging {}", f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
                self.drag_release = true;
            }
            Err(e) => self.status = format!("can't drag it out: {e}"),
        }
    }
}

#[cfg(target_os = "macos")]
fn system(files: &[PathBuf]) -> Result<(), String> {
    mac::start(files)
}

#[cfg(not(target_os = "macos"))]
fn system(_files: &[PathBuf]) -> Result<(), String> {
    Err("dragging out is only on macOS for now".into())
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AnyThread as _, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{
        NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent,
        NSEventModifierFlags, NSEventType, NSWorkspace,
    };
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
    use std::cell::RefCell;
    use std::path::PathBuf;

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DeckardDragSource"]
        struct Source;

        unsafe impl NSObjectProtocol for Source {}

        unsafe impl NSDraggingSource for Source {
            /// A copy, wherever it's dropped: the file stays where it is.
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
                NSDragOperation::Copy
            }
        }
    );

    thread_local! {
        /// The drag's source: the session doesn't keep it.
        static SOURCE: RefCell<Option<Retained<Source>>> = const { RefCell::new(None) };
    }

    pub fn start(files: &[PathBuf]) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        let window =
            app.mainWindow().or_else(|| app.keyWindow()).or_else(|| app.windows().firstObject()).ok_or("no window to drag from")?;
        let view = window.contentView().ok_or("no view to drag from")?;
        let at = window.mouseLocationOutsideOfEventStream();
        // The press (or the move since) the drag begins from: the event being handled if
        // it's the mouse's, else one made where the pointer is.
        let mouse = |e: &Retained<NSEvent>| matches!(e.r#type(), NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged);
        let event = match app.currentEvent().filter(mouse) {
            Some(e) => e,
            None => NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::LeftMouseDragged,
                at,
                NSEventModifierFlags::empty(),
                objc2_foundation::NSProcessInfo::processInfo().systemUptime(),
                window.windowNumber(),
                None,
                0,
                1,
                1.0,
            )
            .ok_or("no mouse press to drag from")?,
        };
        let here = view.convertPoint_fromView(at, None);
        let workspace = NSWorkspace::sharedWorkspace();
        let items: Vec<Retained<NSDraggingItem>> = files
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let path = NSString::from_str(&f.to_string_lossy());
                let url = NSURL::fileURLWithPath(&path);
                let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
                // What's seen under the pointer: the file's icon (each a little off the last).
                let icon = workspace.iconForFile(&path);
                let off = 6.0 * k as f64;
                let frame = NSRect::new(NSPoint::new(here.x - 24.0 + off, here.y - 24.0 - off), NSSize::new(48.0, 48.0));
                // SAFETY: an image is contents a dragging item takes.
                unsafe { item.setDraggingFrame_contents(frame, Some(&icon)) };
                item
            })
            .collect();
        let source: Retained<Source> = unsafe { msg_send![Source::alloc(mtm), init] };
        view.beginDraggingSessionWithItems_event_source(&NSArray::from_retained_slice(&items), &event, ProtocolObject::from_ref(&*source));
        SOURCE.with(|s| *s.borrow_mut() = Some(source));
        Ok(())
    }
}
