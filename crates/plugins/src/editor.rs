//! A plug-in's own editor, in a window of its own (macOS: an `NSWindow` whose content
//! view the plug-in draws into).

use crate::Shared;
use std::cell::Cell;
use std::ffi::c_void;
use vst3::Steinberg::Vst::IEditControllerTrait;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComWrapper};

#[cfg(target_os = "macos")]
use objc2::{MainThreadMarker, rc::Retained};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// Lets the plug-in resize its window. A plug-in usually asks from inside its own event
/// handling: answering there (resizing the window under it, calling its `onSize`) calls
/// back into a plug-in that is still in the middle of a call, which some don't survive
/// (baseview's windows abort the process). So the size is only noted, and applied on the
/// editor's next [`Editor::tick`].
struct Frame {
    #[cfg(target_os = "macos")]
    window: Cell<*const NSWindow>,
    pending: Cell<Option<ViewRect>>,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            #[cfg(target_os = "macos")]
            window: Cell::new(std::ptr::null()),
            pending: Cell::new(None),
        }
    }
}

impl Frame {
    /// Resize the window and the plug-in's view to the size it last asked for, if any.
    fn apply_pending(&self, view: &ComPtr<IPlugView>) {
        let Some(mut r) = self.pending.take() else { return };
        #[cfg(target_os = "macos")]
        unsafe {
            if let Some(w) = self.window.get().as_ref() {
                w.setContentSize(NSSize::new((r.right - r.left) as f64, (r.bottom - r.top) as f64));
            }
        }
        unsafe { view.onSize(&mut r) };
    }
}

impl Class for Frame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for Frame {
    unsafe fn resizeView(&self, view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        if view.is_null() || new_size.is_null() {
            return kInvalidArgument;
        }
        // Noted, applied once the plug-in has returned (see `Frame`).
        self.pending.set(Some(unsafe { *new_size }));
        kResultOk
    }
}

/// An open editor window. Closing the window (or dropping this) closes the editor.
pub struct Editor {
    plugin: Shared,
    view: ComPtr<IPlugView>,
    frame: ComWrapper<Frame>,
    #[cfg(target_os = "macos")]
    window: Retained<NSWindow>,
}

impl Editor {
    /// Open `plugin`'s editor in a window titled `title`. Must be called on the main
    /// thread.
    #[cfg(target_os = "macos")]
    pub fn open(plugin: &Shared, title: &str) -> Result<Editor, String> {
        let mtm = MainThreadMarker::new().ok_or("plug-in windows can only be opened on the main thread")?;
        let view = {
            let p = plugin.lock().map_err(|_| "the plug-in is unavailable")?;
            let c = p.controller().ok_or("the plug-in has no editor")?;
            unsafe { ComPtr::from_raw(c.createView(c"editor".as_ptr())) }.ok_or("the plug-in has no editor")?
        };
        if unsafe { view.isPlatformTypeSupported(kPlatformTypeNSView) } != kResultOk {
            return Err("the plug-in's editor doesn't support macOS views".into());
        }
        let mut size = ViewRect { left: 0, top: 0, right: 640, bottom: 480 };
        unsafe { view.getSize(&mut size) };
        let (w, h) = ((size.right - size.left).max(100) as f64, (size.bottom - size.top).max(60) as f64);
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
        let frame = ComWrapper::new(Frame { window: Cell::new(Retained::as_ptr(&window)), pending: Cell::new(None) });
        unsafe { view.setFrame(frame.as_com_ref::<IPlugFrame>().unwrap().as_ptr()) };
        let content = window.contentView().ok_or("the window has no content view")?;
        let r = unsafe { view.attached(Retained::as_ptr(&content) as *mut c_void, kPlatformTypeNSView) };
        if r != kResultOk {
            window.close();
            return Err(format!("the plug-in's editor could not be opened ({r})"));
        }
        window.makeKeyAndOrderFront(None);
        Ok(Editor { plugin: plugin.clone(), view, frame, window })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn open(_plugin: &Shared, _title: &str) -> Result<Editor, String> {
        Err("plug-in windows are only implemented on macOS".into())
    }

    /// Apply what the plug-in asked for since the last tick (a new size). Call it from
    /// the app's frame, outside any call into the plug-in.
    pub fn tick(&self) {
        self.frame.apply_pending(&self.view);
    }

    /// The window is still open.
    pub fn is_open(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self.window.isVisible();
        #[cfg(not(target_os = "macos"))]
        false
    }

    /// Bring the window to the front.
    pub fn raise(&self) {
        #[cfg(target_os = "macos")]
        self.window.makeKeyAndOrderFront(None);
    }

    pub fn plugin(&self) -> &Shared {
        &self.plugin
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            self.view.removed();
            self.view.setFrame(std::ptr::null_mut());
        }
        #[cfg(target_os = "macos")]
        {
            self.frame.window.set(std::ptr::null());
            self.window.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A plug-in's view that counts the sizes it's given.
    struct View(Arc<AtomicU32>);

    impl Class for View {
        type Interfaces = (IPlugView,);
    }

    impl IPlugViewTrait for View {
        unsafe fn isPlatformTypeSupported(&self, _: FIDString) -> tresult {
            kResultOk
        }
        unsafe fn attached(&self, _: *mut c_void, _: FIDString) -> tresult {
            kResultOk
        }
        unsafe fn removed(&self) -> tresult {
            kResultOk
        }
        unsafe fn onWheel(&self, _: f32) -> tresult {
            kResultOk
        }
        unsafe fn onKeyDown(&self, _: char16, _: int16, _: int16) -> tresult {
            kResultOk
        }
        unsafe fn onKeyUp(&self, _: char16, _: int16, _: int16) -> tresult {
            kResultOk
        }
        unsafe fn getSize(&self, _: *mut ViewRect) -> tresult {
            kResultOk
        }
        unsafe fn onSize(&self, _: *mut ViewRect) -> tresult {
            self.0.fetch_add(1, Ordering::SeqCst);
            kResultOk
        }
        unsafe fn onFocus(&self, _: TBool) -> tresult {
            kResultOk
        }
        unsafe fn setFrame(&self, _: *mut IPlugFrame) -> tresult {
            kResultOk
        }
        unsafe fn canResize(&self) -> tresult {
            kResultTrue
        }
        unsafe fn checkSizeConstraint(&self, _: *mut ViewRect) -> tresult {
            kResultOk
        }
    }

    /// A plug-in asking to resize (often from inside its own event handling) isn't
    /// called back while it's in that call: calling into its view there (and the window
    /// resizing under it) is what crashed a plug-in built on baseview. The size is
    /// applied on the editor's next tick, once.
    #[test]
    fn a_resize_waits_until_the_plugin_has_returned() {
        let sized = Arc::new(AtomicU32::new(0));
        let view = ComWrapper::new(View(sized.clone()));
        let view = view.to_com_ptr::<IPlugView>().unwrap();
        let frame = ComWrapper::new(Frame::default());
        let mut r = ViewRect { left: 0, top: 0, right: 300, bottom: 200 };
        assert_eq!(unsafe { frame.resizeView(view.as_ptr(), &mut r) }, kResultOk);
        assert_eq!(sized.load(Ordering::SeqCst), 0, "not during the plug-in's call");
        frame.apply_pending(&view);
        assert_eq!(sized.load(Ordering::SeqCst), 1);
        frame.apply_pending(&view);
        assert_eq!(sized.load(Ordering::SeqCst), 1, "once");
    }
}
