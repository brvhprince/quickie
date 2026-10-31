use tauri::{AppHandle, PhysicalPosition, PhysicalSize, WebviewWindow};

const BUBBLE_SIZE: (f64, f64) = (64.0, 64.0);
const PANEL_SIZE: (f64, f64) = (360.0, 560.0);

#[tauri::command]
pub fn set_expanded(app: AppHandle, window: WebviewWindow, expanded: bool) -> Result<(), String> {
    let size = if expanded { PANEL_SIZE } else { BUBBLE_SIZE };
    resize_anchored(&window, size).map_err(|e| e.to_string())?;
    focus(&app, &window, expanded).map_err(|e| e.to_string())
}

/// Gives the open shelf the keyboard, and hands it back when the shelf closes.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
fn focus(app: &AppHandle, window: &WebviewWindow, expanded: bool) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        if let Ok(panel) = app.get_webview_panel(window.label()) {
            if expanded {
                panel.make_key_window();
            } else {
                panel.resign_key_window();
            }
            return Ok(());
        }
    }
    if expanded {
        window.set_focus()?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(ShelfPanel {
        config: {
            can_become_key_window: true,
            is_floating_panel: true
        }
    })
}

/// On macOS, turns the window into a non-activating panel, like Spotlight: you
/// can click the bubble and type into the shelf while the app you were using
/// stays frontmost, and keystrokes go back to it when the shelf closes.
#[cfg(target_os = "macos")]
pub fn make_panel(window: &WebviewWindow) -> tauri::Result<()> {
    use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};
    let panel = window.to_panel::<ShelfPanel>()?;
    panel.set_level(PanelLevel::Floating.value());
    // If AppKit refuses, it's still a working floating window, just one that takes focus.
    if let Err(err) = panel.set_style_mask(StyleMask::empty().borderless().nonactivating_panel().into()) {
        tracing::warn!("could not make the bubble non-activating: {err:?}");
    }
    // Stay put across Spaces and over full-screen apps.
    panel.set_collection_behavior(
        CollectionBehavior::new().can_join_all_spaces().full_screen_auxiliary().stationary().into(),
    );
    Ok(())
}

/// Resizes while keeping the corner nearest the screen edge fixed, so the bubble
/// opens into the screen instead of off it.
fn resize_anchored(window: &WebviewWindow, (w, h): (f64, f64)) -> tauri::Result<()> {
    let scale = window.scale_factor()?;
    let new = PhysicalSize::new((w * scale).round() as i32, (h * scale).round() as i32);
    let pos = window.outer_position()?;
    let old = window.outer_size()?;
    let (old_w, old_h) = (old.width as i32, old.height as i32);
    let Some(monitor) = window.current_monitor()? else {
        return window.set_size(PhysicalSize::new(new.width as u32, new.height as u32));
    };
    let area = monitor.work_area();
    let (mx, my) = (area.position.x, area.position.y);
    let (mw, mh) = (area.size.width as i32, area.size.height as i32);

    let anchor_right = pos.x + old_w / 2 > mx + mw / 2;
    let anchor_bottom = pos.y + old_h / 2 > my + mh / 2;
    let x = if anchor_right { pos.x + old_w - new.width } else { pos.x };
    let y = if anchor_bottom { pos.y + old_h - new.height } else { pos.y };
    let x = x.min(mx + mw - new.width).max(mx);
    let y = y.min(my + mh - new.height).max(my);

    window.set_size(PhysicalSize::new(new.width as u32, new.height as u32))?;
    window.set_position(PhysicalPosition::new(x, y))
}

/// Parks the bubble on the right edge, a bit above the middle.
pub fn place_bubble(window: &WebviewWindow) -> tauri::Result<()> {
    if let Some(monitor) = window.current_monitor()? {
        let area = monitor.work_area();
        let size = window.outer_size()?;
        let margin = (12.0 * monitor.scale_factor()) as i32;
        let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
        let y = area.position.y + (area.size.height as f64 * 0.4) as i32;
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    window.show()
}
