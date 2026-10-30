//! Where the pointer is while files are dragged over the window, in the window's
//! points. The windowing layer (winit 0.30) says files are hovering or dropped but not
//! where on macOS and Windows, and the window gets no pointer moves meanwhile, so the
//! system is asked. Elsewhere (Linux), the last pointer position egui knows.

use eframe::egui::{Context, Pos2};

pub fn pointer(ctx: &Context) -> Option<Pos2> {
    system(ctx).or_else(|| ctx.input(|i| i.pointer.latest_pos()))
}

/// AppKit: the main window's own idea of the pointer (its content's bottom-left origin).
#[cfg(target_os = "macos")]
fn system(_ctx: &Context) -> Option<Pos2> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let window = app.mainWindow().or_else(|| app.windows().firstObject())?;
    let at = window.mouseLocationOutsideOfEventStream();
    let height = window.contentView()?.frame().size.height;
    Some(Pos2::new(at.x as f32, (height - at.y) as f32))
}

/// Win32: the cursor on the screen (pixels), from the window's inner corner.
#[cfg(windows)]
fn system(ctx: &Context) -> Option<Pos2> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: GetCursorPos only writes the point it's given.
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return None;
    }
    let inner = ctx.input(|i| i.viewport().inner_rect)?;
    let ppp = ctx.pixels_per_point();
    Some(Pos2::new(p.x as f32 / ppp - inner.min.x, p.y as f32 / ppp - inner.min.y))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn system(_ctx: &Context) -> Option<Pos2> {
    None
}
