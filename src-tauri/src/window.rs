use tauri::WebviewWindow;

/// Remove native decoration artifacts and preserve the rounded transparent
/// shell used by the desk client.
#[cfg(target_os = "windows")]
pub fn remove_native_border(window: &WebviewWindow) -> tauri::Result<()> {
    // DWM's shadow on transparent frameless windows can render a bright top
    // edge and a dark bottom edge. Keep the desk client's clean borderless
    // surface and let the WebView shell provide the restrained visual depth.
    window.set_shadow(false)?;
    refresh_native_rounding(window)?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn refresh_native_rounding(window: &WebviewWindow) -> tauri::Result<()> {
    use std::{ffi::c_void, mem::size_of};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL,
    };

    let hwnd = window.hwnd()?.0 as *mut c_void;
    let border_color = DWMWA_COLOR_NONE;
    let corner_preference = DWMWCP_ROUNDSMALL;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &border_color as *const u32 as *const c_void,
            size_of::<u32>() as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &corner_preference as *const i32 as *const c_void,
            size_of::<i32>() as u32,
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn remove_native_border(window: &WebviewWindow) -> tauri::Result<()> {
    use objc2_app_kit::{NSColor, NSWindow};

    let native_window = window.ns_window()? as *const NSWindow;
    if native_window.is_null() {
        return Ok(());
    }

    unsafe {
        let native_window = &*native_window;
        native_window.setOpaque(false);
        native_window.setBackgroundColor(Some(&NSColor::clearColor()));
        if let Some(content_view) = native_window.contentView() {
            content_view.setWantsLayer(true);
            if let Some(layer) = content_view.layer() {
                layer.setCornerRadius(8.0);
                layer.setMasksToBounds(true);
            }
        }
    }
    window.set_shadow(true)?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn refresh_native_rounding(_window: &WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn remove_native_border(_window: &WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn refresh_native_rounding(_window: &WebviewWindow) -> tauri::Result<()> {
    Ok(())
}
