use tauri::{Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

const MIN_WIDTH: f64 = 320.0;
const MIN_HEIGHT: f64 = 605.0;
const MAX_WIDTH: f64 = 420.0;
const MAX_HEIGHT: f64 = 794.0;
const SHADOW_GUTTER: f64 = 16.0;
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
    let fitting_maximum_width = (work_width * MAX_WORK_AREA_RATIO - SHADOW_GUTTER * 2.0)
        .min((work_height * MAX_WORK_AREA_RATIO - SHADOW_GUTTER * 2.0) * TARGET_ASPECT_RATIO)
        .max(minimum_width);
    let maximum_width = absolute_maximum_width.min(fitting_maximum_width);
    let shell_width = width_from_area.clamp(minimum_width, maximum_width);
    let shell_height = shell_width / TARGET_ASPECT_RATIO;
    let width = shell_width + SHADOW_GUTTER * 2.0;
    let height = shell_height + SHADOW_GUTTER * 2.0;
    let right_gap = work_width * RIGHT_GAP_RATIO;
    let x = (work_x + work_width - width - right_gap + SHADOW_GUTTER).max(work_x);
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
    let shadow_gutter = to_physical(SHADOW_GUTTER) as i32;
    window.set_position(PhysicalPosition::new(
        work.position.x + work.size.width as i32 - outer_size.width as i32 - right_gap
            + shadow_gutter,
        work.position.y + (work.size.height as i32 - outer_size.height as i32) / 2,
    ))?;
    refresh_native_rounding(window)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_shell_ratio_and_reserves_shadow_gutter() {
        let geometry = calculate_geometry(0.0, 0.0, 1920.0, 1040.0);
        let shell_width = geometry.width - SHADOW_GUTTER * 2.0;
        let shell_height = geometry.height - SHADOW_GUTTER * 2.0;

        assert!((shell_width - 352.7).abs() < 2.0);
        assert!((shell_height - 666.1).abs() < 2.0);
        assert!((shell_width / shell_height - TARGET_ASPECT_RATIO).abs() < 0.001);
        assert!((geometry.x - (1920.0 - shell_width - 192.0 - SHADOW_GUTTER)).abs() < 0.1);
        assert!((geometry.y - ((1040.0 - geometry.height) / 2.0)).abs() < 0.1);
    }

    #[test]
    fn keeps_minimum_shell_size_on_small_work_areas() {
        let geometry = calculate_geometry(-100.0, 20.0, 500.0, 340.0);
        assert!(
            (geometry.width
                - MIN_WIDTH.max(MIN_HEIGHT * TARGET_ASPECT_RATIO)
                - SHADOW_GUTTER * 2.0)
                .abs()
                < 0.1
        );
        assert!((geometry.height - MIN_HEIGHT - SHADOW_GUTTER * 2.0).abs() < 0.1);
        assert!((geometry.x - 13.7).abs() < 0.1);
        assert_eq!(geometry.y, 20.0);
    }

    #[test]
    fn caps_large_displays_at_maximum_shell_size() {
        let geometry = calculate_geometry(0.0, 0.0, 3840.0, 2120.0);
        assert_eq!(geometry.width, MAX_WIDTH + SHADOW_GUTTER * 2.0);
        assert!(
            (geometry.height - (MAX_WIDTH / TARGET_ASPECT_RATIO) - SHADOW_GUTTER * 2.0).abs() < 0.1
        );
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn remove_native_border(_window: &WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn refresh_native_rounding(_window: &WebviewWindow) -> tauri::Result<()> {
    Ok(())
}
