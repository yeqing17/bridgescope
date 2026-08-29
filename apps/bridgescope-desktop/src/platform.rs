//! Native window chrome tweaks that egui does not expose.
//!
//! The app draws its own title bar on an undecorated window, which Windows
//! renders with sharp corners and no shadow. Two read-only DWM calls fix
//! that: the Windows 11 rounded-corner preference, and a 1px frame extension
//! at the bottom so DWM paints its drop shadow (a bottom-only margin avoids
//! the 1px top line winit's own undecorated-shadow helper produces). Both are
//! best-effort; on Windows 10 the corner attribute is unsupported and the
//! calls simply fail silently. On other platforms this module is a no-op.

// The workspace denies `unsafe_code`; this module is the single, reviewed
// opt-in (two DWM composition calls on our own window).
#![allow(unsafe_code)]

/// Requests rounded corners and a drop shadow for the app's main window.
pub fn apply_window_chrome(creation_context: &eframe::CreationContext<'_>) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = creation_context.window_handle()
            && let RawWindowHandle::Win32(win) = handle.as_raw()
        {
            rounded_corners_and_shadow(win.hwnd.get() as *mut std::ffi::c_void);
        }
    }
    #[cfg(not(windows))]
    let _ = creation_context;
}

#[cfg(windows)]
fn rounded_corners_and_shadow(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
        DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;

    // SAFETY: both calls only describe how DWM should composite our own
    // window (corner shape + frame margins); they neither read nor write
    // process data. Failures (e.g. Windows 10) leave the square look.
    unsafe {
        let preference = DWMWCP_ROUND;
        let result = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            std::ptr::from_ref(&preference).cast(),
            u32::try_from(size_of_val(&preference)).unwrap_or_default(),
        );
        if result != 0 {
            tracing::debug!("DwmSetWindowAttribute(round corners) failed: {result}");
        }
        let margins = MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 1,
        };
        let result = DwmExtendFrameIntoClientArea(hwnd, &raw const margins);
        if result != 0 {
            tracing::debug!("DwmExtendFrameIntoClientArea(shadow) failed: {result}");
        }
    }
}
