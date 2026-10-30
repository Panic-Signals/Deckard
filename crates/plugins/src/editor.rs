//! A plug-in's own editor, in a window of its own: on macOS an `NSWindow` whose content
//! view the plug-in draws into; on Linux an X11 window the plug-in puts its own window
//! inside, served by the host's run loop ([`crate::runloop`]); on Windows a window whose
//! messages the app's own loop dispatches.

use crate::Shared;
use std::cell::Cell;
#[cfg(any(target_os = "macos", target_os = "linux", windows, test))]
use std::ffi::c_void;
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
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
///
/// On Linux the frame is also the plug-in's run loop.
struct Frame {
    #[cfg(target_os = "macos")]
    window: Cell<*const NSWindow>,
    pending: Cell<Option<ViewRect>>,
    #[cfg(target_os = "linux")]
    run_loop: crate::runloop::RunLoop,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            #[cfg(target_os = "macos")]
            window: Cell::new(std::ptr::null()),
            pending: Cell::new(None),
            #[cfg(target_os = "linux")]
            run_loop: Default::default(),
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
    #[cfg(target_os = "linux")]
    type Interfaces = (IPlugFrame, Linux::IRunLoop);
    #[cfg(not(target_os = "linux"))]
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

#[cfg(target_os = "linux")]
impl Linux::IRunLoopTrait for Frame {
    unsafe fn registerEventHandler(&self, handler: *mut Linux::IEventHandler, fd: Linux::FileDescriptor) -> tresult {
        if unsafe { self.run_loop.register_handler(handler, fd) } { kResultOk } else { kInvalidArgument }
    }
    unsafe fn unregisterEventHandler(&self, handler: *mut Linux::IEventHandler) -> tresult {
        if self.run_loop.unregister_handler(handler) { kResultOk } else { kInvalidArgument }
    }
    unsafe fn registerTimer(&self, handler: *mut Linux::ITimerHandler, milliseconds: Linux::TimerInterval) -> tresult {
        if unsafe { self.run_loop.register_timer(handler, milliseconds) } { kResultOk } else { kInvalidArgument }
    }
    unsafe fn unregisterTimer(&self, handler: *mut Linux::ITimerHandler) -> tresult {
        if self.run_loop.unregister_timer(handler) { kResultOk } else { kInvalidArgument }
    }
}

/// An open editor window. Closing the window (or dropping this) closes the editor.
pub struct Editor {
    plugin: Shared,
    view: ComPtr<IPlugView>,
    frame: ComWrapper<Frame>,
    #[cfg(target_os = "macos")]
    window: Retained<NSWindow>,
    #[cfg(target_os = "linux")]
    window: x11::Window,
    #[cfg(windows)]
    window: win32::Window,
}

/// The plug-in's editor view for `platform`, and its size.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn create_view(plugin: &Shared, platform: FIDString, platform_name: &str) -> Result<(ComPtr<IPlugView>, i32, i32), String> {
    let view = {
        let p = plugin.lock().map_err(|_| "the plug-in is unavailable")?;
        let c = p.controller().ok_or("the plug-in has no editor")?;
        unsafe { ComPtr::from_raw(c.createView(c"editor".as_ptr())) }.ok_or("the plug-in has no editor")?
    };
    if unsafe { view.isPlatformTypeSupported(platform) } != kResultOk {
        return Err(format!("the plug-in's editor doesn't support {platform_name} windows"));
    }
    let mut size = ViewRect { left: 0, top: 0, right: 640, bottom: 480 };
    unsafe { view.getSize(&mut size) };
    Ok((view, (size.right - size.left).max(100), (size.bottom - size.top).max(60)))
}

impl Editor {
    /// Open `plugin`'s editor in a window titled `title`. Must be called on the main
    /// thread.
    #[cfg(target_os = "macos")]
    pub fn open(plugin: &Shared, title: &str) -> Result<Editor, String> {
        let mtm = MainThreadMarker::new().ok_or("plug-in windows can only be opened on the main thread")?;
        let (view, w, h) = create_view(plugin, kPlatformTypeNSView, "macOS")?;
        let (w, h) = (w as f64, h as f64);
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

    /// Open `plugin`'s editor in an X11 window (through XWayland on a Wayland desktop).
    /// Must be called on the main thread.
    #[cfg(target_os = "linux")]
    pub fn open(plugin: &Shared, title: &str) -> Result<Editor, String> {
        if !crate::on_main_thread() {
            return Err("plug-in windows can only be opened on the main thread".into());
        }
        let (view, w, h) = create_view(plugin, kPlatformTypeX11EmbedWindowID, "X11")?;
        let resizable = unsafe { view.canResize() } == kResultTrue;
        let window = x11::Window::open(title, w.min(u16::MAX as i32) as u16, h.min(u16::MAX as i32) as u16, resizable)?;
        let frame = ComWrapper::new(Frame::default());
        unsafe { view.setFrame(frame.as_com_ref::<IPlugFrame>().unwrap().as_ptr()) };
        let r = unsafe { view.attached(window.id as usize as *mut c_void, kPlatformTypeX11EmbedWindowID) };
        if r != kResultOk {
            unsafe { view.setFrame(std::ptr::null_mut()) };
            frame.run_loop.clear();
            window.close();
            return Err(format!("the plug-in's editor could not be opened ({r})"));
        }
        Ok(Editor { plugin: plugin.clone(), view, frame, window })
    }

    /// Open `plugin`'s editor in a window of its own. Must be called on the main thread
    /// (the one that runs the app's message loop, which dispatches the window's
    /// messages).
    #[cfg(windows)]
    pub fn open(plugin: &Shared, title: &str) -> Result<Editor, String> {
        if !crate::on_main_thread() {
            return Err("plug-in windows can only be opened on the main thread".into());
        }
        let (view, w, h) = create_view(plugin, kPlatformTypeHWND, "Windows")?;
        let resizable = unsafe { view.canResize() } == kResultTrue;
        let window = win32::Window::open(title, w, h, resizable)?;
        let frame = ComWrapper::new(Frame::default());
        unsafe { view.setFrame(frame.as_com_ref::<IPlugFrame>().unwrap().as_ptr()) };
        scale_view(&view, window.dpi());
        let r = unsafe { view.attached(window.hwnd as *mut c_void, kPlatformTypeHWND) };
        if r != kResultOk {
            unsafe { view.setFrame(std::ptr::null_mut()) };
            window.close();
            return Err(format!("the plug-in's editor could not be opened ({r})"));
        }
        window.show();
        Ok(Editor { plugin: plugin.clone(), view, frame, window })
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    pub fn open(_plugin: &Shared, _title: &str) -> Result<Editor, String> {
        Err("plug-in windows are not implemented on this system yet".into())
    }

    /// Dispatch the window's pending messages. Only for programs without a message loop
    /// of their own (the app's loop dispatches them itself).
    #[cfg(windows)]
    pub fn pump_messages(&self) {
        self.window.pump_messages();
    }

    /// Serve the plug-in's editor: on Linux its run loop and its window's events; and
    /// apply what the plug-in asked for since the last tick (a new size). Call it from
    /// the app's frame, outside any call into the plug-in.
    pub fn tick(&self) {
        #[cfg(target_os = "linux")]
        {
            self.frame.run_loop.poll();
            if let Some((w, h)) = self.window.pump()
                && self.window.is_open()
            {
                let mut r = ViewRect { left: 0, top: 0, right: w as i32, bottom: h as i32 };
                unsafe { self.view.onSize(&mut r) };
            }
            if let Some(r) = self.frame.pending.get() {
                self.window
                    .resize((r.right - r.left).clamp(1, u16::MAX as i32) as u16, (r.bottom - r.top).clamp(1, u16::MAX as i32) as u16);
            }
        }
        #[cfg(windows)]
        {
            if let Some(dpi) = self.window.take_dpi() {
                scale_view(&self.view, dpi);
            }
            if let Some((w, h)) = self.window.take_resized()
                && self.window.is_open()
            {
                let mut r = ViewRect { left: 0, top: 0, right: w, bottom: h };
                unsafe { self.view.onSize(&mut r) };
            }
            if let Some(r) = self.frame.pending.get() {
                self.window.resize((r.right - r.left).max(1), (r.bottom - r.top).max(1));
            }
        }
        self.frame.apply_pending(&self.view);
    }

    /// The window is still open.
    pub fn is_open(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self.window.isVisible();
        #[cfg(any(target_os = "linux", windows))]
        return self.window.is_open();
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        false
    }

    /// Bring the window to the front.
    pub fn raise(&self) {
        #[cfg(target_os = "macos")]
        self.window.makeKeyAndOrderFront(None);
        #[cfg(any(target_os = "linux", windows))]
        self.window.raise();
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
        #[cfg(target_os = "linux")]
        {
            // Nothing reaches the plug-in after its view is gone.
            self.frame.run_loop.clear();
            self.window.close();
        }
        #[cfg(windows)]
        self.window.close();
    }
}

/// Tell the view the window's scale (`dpi` / 96), if it wants to know.
#[cfg(windows)]
fn scale_view(view: &ComPtr<IPlugView>, dpi: u32) {
    if let Some(s) = view.cast::<IPlugViewContentScaleSupport>() {
        unsafe { s.setContentScaleFactor(dpi as f32 / 96.0) };
    }
}

/// A top-level window of our own for the plug-in to put its editor in. Its messages are
/// dispatched by whatever message loop the thread runs (the app's), or by
/// [`Window::pump_messages`].
#[cfg(windows)]
mod win32 {
    use std::cell::Cell;
    use std::sync::Once;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    /// What the window procedure noted, for the next tick.
    struct State {
        open: Cell<bool>,
        resized: Cell<Option<(i32, i32)>>,
        dpi: Cell<Option<u32>>,
    }

    pub struct Window {
        pub hwnd: HWND,
        state: Box<State>,
        size: Cell<(i32, i32)>,
    }

    static CLASS: Once = Once::new();
    /// "DeckardPlugin"
    const CLASS_NAME: &[u16] = &[0x44, 0x65, 0x63, 0x6b, 0x61, 0x72, 0x64, 0x50, 0x6c, 0x75, 0x67, 0x69, 0x6e, 0];

    fn low(v: isize) -> i32 {
        (v & 0xffff) as i16 as i32
    }

    fn high(v: isize) -> i32 {
        ((v >> 16) & 0xffff) as i16 as i32
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let state = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State).as_ref() };
        match msg {
            WM_CLOSE => {
                // Hidden now, destroyed by `close` once the plug-in's view is removed.
                if let Some(s) = state {
                    s.open.set(false);
                }
                unsafe { ShowWindow(hwnd, SW_HIDE) };
                0
            }
            WM_SIZE => {
                if let Some(s) = state {
                    s.resized.set(Some((low(lp), high(lp))));
                }
                0
            }
            WM_DPICHANGED => {
                if let Some(r) = unsafe { (lp as *const RECT).as_ref() } {
                    unsafe {
                        SetWindowPos(
                            hwnd,
                            std::ptr::null_mut(),
                            r.left,
                            r.top,
                            r.right - r.left,
                            r.bottom - r.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        )
                    };
                }
                if let Some(s) = state {
                    s.dpi.set(Some(high(wp as isize) as u32));
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    impl Window {
        pub fn open(title: &str, w: i32, h: i32, resizable: bool) -> Result<Window, String> {
            let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
            CLASS.call_once(|| {
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(window_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: instance,
                    hIcon: std::ptr::null_mut(),
                    hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
                    hbrBackground: std::ptr::null_mut(),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: CLASS_NAME.as_ptr(),
                };
                unsafe { RegisterClassW(&class) };
            });
            let mut style = WS_OVERLAPPEDWINDOW;
            if !resizable {
                style &= !(WS_THICKFRAME | WS_MAXIMIZEBOX);
            }
            let mut rect = RECT { left: 0, top: 0, right: w, bottom: h };
            unsafe { AdjustWindowRectEx(&mut rect, style, 0, 0) };
            let title = wide(title);
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    CLASS_NAME.as_ptr(),
                    title.as_ptr(),
                    style,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                )
            };
            if hwnd.is_null() {
                return Err("the plug-in's window could not be made".into());
            }
            let state = Box::new(State { open: Cell::new(true), resized: Cell::new(None), dpi: Cell::new(None) });
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const State as isize) };
            Ok(Window { hwnd, state, size: Cell::new((w, h)) })
        }

        pub fn show(&self) {
            unsafe { ShowWindow(self.hwnd, SW_SHOW) };
            // Showing sized the client area: not a resize to pass on.
            self.state.resized.take();
        }

        pub fn dpi(&self) -> u32 {
            match unsafe { GetDpiForWindow(self.hwnd) } {
                0 => 96,
                d => d,
            }
        }

        /// The client size, if the user changed it since the last call.
        pub fn take_resized(&self) -> Option<(i32, i32)> {
            let (w, h) = self.state.resized.take()?;
            if (w, h) == self.size.get() || w <= 0 || h <= 0 {
                return None;
            }
            self.size.set((w, h));
            Some((w, h))
        }

        /// The new DPI, if the window moved to a screen with another.
        pub fn take_dpi(&self) -> Option<u32> {
            self.state.dpi.take()
        }

        /// Resize the client area (the plug-in asked).
        pub fn resize(&self, w: i32, h: i32) {
            if (w, h) == self.size.get() {
                return;
            }
            self.size.set((w, h));
            let style = unsafe { GetWindowLongPtrW(self.hwnd, GWL_STYLE) } as u32;
            let mut rect = RECT { left: 0, top: 0, right: w, bottom: h };
            unsafe {
                AdjustWindowRectEx(&mut rect, style, 0, 0);
                SetWindowPos(
                    self.hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            self.state.resized.take();
        }

        /// Dispatch this thread's pending messages (for programs without a loop).
        pub fn pump_messages(&self) {
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        pub fn is_open(&self) -> bool {
            self.state.open.get()
        }

        pub fn raise(&self) {
            unsafe {
                ShowWindow(self.hwnd, SW_SHOW);
                SetForegroundWindow(self.hwnd);
            }
        }

        pub fn close(&self) {
            self.state.open.set(false);
            if unsafe { IsWindow(self.hwnd) } != 0 {
                unsafe {
                    SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                    DestroyWindow(self.hwnd);
                }
            }
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            self.close();
        }
    }
}

/// A top-level X11 window of our own, on its own connection, for the plug-in to embed
/// its editor in.
#[cfg(target_os = "linux")]
mod x11 {
    use std::cell::Cell;
    use x11rb::connection::Connection;
    use x11rb::properties::WmSizeHints;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::*;
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    pub struct Window {
        conn: RustConnection,
        pub id: u32,
        wm_delete: Atom,
        resizable: bool,
        open: Cell<bool>,
        size: Cell<(u16, u16)>,
    }

    impl Window {
        pub fn open(title: &str, w: u16, h: u16, resizable: bool) -> Result<Window, String> {
            let (conn, screen) = x11rb::connect(None).map_err(|e| format!("no X display for the plug-in's window ({e})"))?;
            let screen = &conn.setup().roots[screen];
            let x = |e: x11rb::errors::ConnectionError| format!("the plug-in's window could not be made ({e})");
            let id = conn.generate_id().map_err(|e| format!("the plug-in's window could not be made ({e})"))?;
            let aux = CreateWindowAux::new().background_pixel(screen.black_pixel).event_mask(EventMask::STRUCTURE_NOTIFY);
            conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                id,
                screen.root,
                0,
                0,
                w,
                h,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &aux,
            )
            .map_err(x)?;
            let atom = |name: &[u8]| {
                let fail = |e: &dyn std::fmt::Display| format!("X atom {}: {e}", String::from_utf8_lossy(name));
                conn.intern_atom(false, name).map_err(|e| fail(&e))?.reply().map(|r| r.atom).map_err(|e| fail(&e))
            };
            let wm_protocols = atom(b"WM_PROTOCOLS")?;
            let wm_delete = atom(b"WM_DELETE_WINDOW")?;
            let net_name = atom(b"_NET_WM_NAME")?;
            let utf8 = atom(b"UTF8_STRING")?;
            let net_pid = atom(b"_NET_WM_PID")?;
            conn.change_property8(PropMode::REPLACE, id, AtomEnum::WM_NAME, AtomEnum::STRING, title.as_bytes()).map_err(x)?;
            conn.change_property8(PropMode::REPLACE, id, net_name, utf8, title.as_bytes()).map_err(x)?;
            conn.change_property8(PropMode::REPLACE, id, AtomEnum::WM_CLASS, AtomEnum::STRING, b"deckard\0DECKARD\0").map_err(x)?;
            conn.change_property32(PropMode::REPLACE, id, wm_protocols, AtomEnum::ATOM, &[wm_delete]).map_err(x)?;
            conn.change_property32(PropMode::REPLACE, id, net_pid, AtomEnum::CARDINAL, &[std::process::id()]).map_err(x)?;
            let window = Window { conn, id, wm_delete, resizable, open: Cell::new(true), size: Cell::new((w, h)) };
            window.set_hints(w, h);
            window.conn.map_window(id).map_err(x)?;
            window.conn.flush().map_err(x)?;
            Ok(window)
        }

        /// Fix the size (a plug-in that can't resize), or just a floor.
        fn set_hints(&self, w: u16, h: u16) {
            let mut hints = WmSizeHints::new();
            hints.min_size = Some(if self.resizable { (100, 60) } else { (w as i32, h as i32) });
            if !self.resizable {
                hints.max_size = Some((w as i32, h as i32));
            }
            let _ = hints.set_normal_hints(&self.conn, self.id);
        }

        /// Handle the window's events. The size, when the user resized it.
        pub fn pump(&self) -> Option<(u16, u16)> {
            let mut resized = None;
            while let Ok(Some(ev)) = self.conn.poll_for_event() {
                match ev {
                    Event::ClientMessage(m) if m.window == self.id && m.data.as_data32()[0] == self.wm_delete => self.open.set(false),
                    Event::DestroyNotify(d) if d.window == self.id => self.open.set(false),
                    Event::ConfigureNotify(c) if c.window == self.id && (c.width, c.height) != self.size.get() => {
                        self.size.set((c.width, c.height));
                        resized = Some((c.width, c.height));
                    }
                    _ => {}
                }
            }
            resized
        }

        /// Resize the window (the plug-in asked).
        pub fn resize(&self, w: u16, h: u16) {
            if (w, h) == self.size.get() {
                return;
            }
            self.size.set((w, h));
            self.set_hints(w, h);
            let _ = self.conn.configure_window(self.id, &ConfigureWindowAux::new().width(w as u32).height(h as u32));
            let _ = self.conn.flush();
        }

        pub fn is_open(&self) -> bool {
            self.open.get()
        }

        pub fn raise(&self) {
            let _ = self.conn.map_window(self.id);
            let _ = self.conn.configure_window(self.id, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
            let _ = self.conn.flush();
        }

        pub fn close(&self) {
            if self.open.replace(false) {
                let _ = self.conn.destroy_window(self.id);
                let _ = self.conn.flush();
            }
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            self.close();
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

    /// On Linux the frame is the plug-in's run loop too: it must answer a query for it.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_frame_is_a_run_loop() {
        let frame = ComWrapper::new(Frame::default());
        assert!(frame.to_com_ptr::<Linux::IRunLoop>().is_some());
    }
}
