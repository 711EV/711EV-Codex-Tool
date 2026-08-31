use tauri::{Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

const MIN_WIDTH: f64 = 320.0;
const MIN_HEIGHT: f64 = 605.0;
const MAX_WIDTH: f64 = 420.0;
const MAX_HEIGHT: f64 = 794.0;
const TARGET_ASPECT_RATIO: f64 = 9.0 / 17.0;
const SCREEN_AREA_RATIO: f64 = 1.0 / 8.5;
const RIGHT_GAP_RATIO: f64 = 0.10;
const MAX_WORK_AREA_RATIO: f64 = 0.90;

#[derive(Debug, Clone, Copy, PartialEq)]
struct LogicalGeometry {
    width: f64,
    height: f64,
    x: f64,
    y: f64,
}

fn calculate_geometry(
    work_x: f64,
    work_y: f64,
    work_width: f64,
    work_height: f64,
) -> LogicalGeometry {
    let target_area = work_width * work_height * SCREEN_AREA_RATIO;
    let width_from_area = (target_area * TARGET_ASPECT_RATIO).sqrt();
    let minimum_width = MIN_WIDTH.max(MIN_HEIGHT * TARGET_ASPECT_RATIO);
    let absolute_maximum_width = MAX_WIDTH.min(MAX_HEIGHT * TARGET_ASPECT_RATIO);
    // 工作区限制只在不低于最小尺寸时生效。极小工作区也必须保持最小尺寸，
    // 允许窗口向工作区边界外延伸，避免文字和控件被无限压缩。
    let fitting_maximum_width = (work_width * MAX_WORK_AREA_RATIO)
        .min(work_height * MAX_WORK_AREA_RATIO * TARGET_ASPECT_RATIO)
        .max(minimum_width);
    let maximum_width = absolute_maximum_width.min(fitting_maximum_width);
    let width = width_from_area.clamp(minimum_width, maximum_width);
    let height = width / TARGET_ASPECT_RATIO;
    let right_gap = work_width * RIGHT_GAP_RATIO;
    let x = (work_x + work_width - width - right_gap).max(work_x);
    let y = work_y + ((work_height - height) / 2.0).max(0.0);

    LogicalGeometry {
        width,
        height,
        x,
        y,
    }
}

fn monitor_for_window(window: &WebviewWindow) -> tauri::Result<Option<Monitor>> {
    window
        .current_monitor()?
        .map_or_else(|| window.primary_monitor(), |monitor| Ok(Some(monitor)))
}

fn apply_geometry(window: &WebviewWindow, monitor: &Monitor) -> tauri::Result<()> {
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    let logical = calculate_geometry(
        work.position.x as f64 / scale,
        work.position.y as f64 / scale,
        work.size.width as f64 / scale,
        work.size.height as f64 / scale,
    );

    let to_physical = |value: f64| (value * scale).round();
    let locked_width = to_physical(logical.width) as u32;
    let locked_height = to_physical(logical.height) as u32;
    let size = PhysicalSize::new(locked_width, locked_height);
    window.set_size(size)?;
    window.set_min_size(Some(size))?;
    window.set_max_size(Some(size))?;
    let outer_size = window.outer_size()?;
    let right_gap = (work.size.width as f64 * RIGHT_GAP_RATIO).round() as i32;
    window.set_position(PhysicalPosition::new(
        work.position.x + work.size.width as i32 - outer_size.width as i32 - right_gap,
        work.position.y + (work.size.height as i32 - outer_size.height as i32) / 2,
    ))?;
    Ok(())
}

pub fn arrange_initial_window(window: &WebviewWindow) -> tauri::Result<()> {
    if let Some(monitor) = monitor_for_window(window)? {
        apply_geometry(window, &monitor)?;
    }
    Ok(())
}

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
