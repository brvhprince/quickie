mod commands;
mod window;

use std::path::PathBuf;

use quickie_core::{Config, Node};
use tauri::{
    App, AppHandle, Emitter, Manager, RunEvent,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};
use tokio::sync::broadcast::error::RecvError;

/// ⌃⌥Space (Ctrl+Alt+Space on Windows) opens the shelf focused, so ⌘V shares at once.
const TOGGLE_SHORTCUT: (Modifiers, Code) = (Modifiers::CONTROL.union(Modifiers::ALT), Code::Space);

/// `QUICKIE_INSTANCE=b` runs a second, independent device on the same machine.
fn instance() -> Option<String> {
    std::env::var("QUICKIE_INSTANCE").ok().filter(|s| !s.is_empty())
}

fn node_config(app: &App) -> tauri::Result<Config> {
    let mut data_dir = app.path().app_data_dir()?;
    let mut name = None;
    if let Some(instance) = instance() {
        data_dir.push(&instance);
        name = Some(format!("{} ({instance})", quickie_core::default_device_name()));
    }
    Ok(Config { device_name: name, web_root: web_root(app), ..Config::new(data_dir) })
}

/// The phone web client, bundled from `dist-web/`. `QUICKIE_WEB_ROOT` points at
/// a live build instead, for working on it with `pnpm dev:web`.
fn web_root(app: &App) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("QUICKIE_WEB_ROOT") {
        return Some(dir.into());
    }
    let dir = app.path().resource_dir().ok()?.join("web");
    let found = dir.join("index.html").exists();
    if !found {
        tracing::warn!("phone web client not found in {}; run `pnpm build:web`", dir.display());
    }
    found.then_some(dir)
}

fn forward_changes(app: AppHandle, node: Node) {
    let mut changes = node.subscribe();
    tauri::async_runtime::spawn(async move {
        // A lagged receiver just means we coalesced several changes into one emit.
        while let Ok(()) | Err(RecvError::Lagged(_)) = changes.recv().await {
            let _ = app.emit("shelf", node.snapshot());
        }
    });
}

fn show_shelf(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
    }
    let _ = app.emit("toggle-shelf", ());
}

fn build_tray(app: &App) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / Hide Bubble", true, None::<&str>)?;
    let login_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let login = CheckMenuItem::with_id(app, "login", "Open at Login", true, login_enabled, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Quickie", true, Some("CmdOrCtrl+Q"))?;
    let menu = Menu::with_items(app, &[&toggle, &login, &PredefinedMenuItem::separator(app)?, &quit])?;

    let mut tray = TrayIconBuilder::with_id("main").menu(&menu).tooltip("Quickie");
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(move |app, event| match event.id.as_ref() {
        "toggle" => {
            if let Some(window) = app.get_webview_window("main") {
                let visible = window.is_visible().unwrap_or(false);
                let _ = if visible { window.hide() } else { window.show() };
            }
        }
        "login" => {
            let autolaunch = app.autolaunch();
            let enable = !autolaunch.is_enabled().unwrap_or(false);
            let result = if enable { autolaunch.enable() } else { autolaunch.disable() };
            if let Err(err) = result {
                tracing::error!("could not change open-at-login: {err}");
            }
            let _ = login.set_checked(autolaunch.is_enabled().unwrap_or(false));
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt().with_env_filter("quickie=info,quickie_core=info").init();

    let mut builder = tauri::Builder::default();
    // Test instances skip these: they'd fight the main instance for the lock and hotkey.
    if instance().is_none() {
        let (mods, key) = TOGGLE_SHORTCUT;
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _, _| show_shelf(app)))
            .plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_shortcut(Shortcut::new(Some(mods), key))
                    .expect("valid shortcut")
                    .with_handler(|app, _, event| {
                        if event.state == ShortcutState::Pressed {
                            show_shelf(app);
                        }
                    })
                    .build(),
            );
    }

    let app = builder
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::snapshot,
            commands::share_text,
            commands::mount_paths,
            commands::unmount,
            commands::fetch,
            commands::open_item,
            commands::save_item,
            commands::request_pairing,
            commands::pair,
            commands::cancel_pairing,
            commands::create_web_invite,
            commands::cancel_web_invite,
            commands::unpair,
            window::set_expanded,
        ])
        .setup(|app| {
            // A floating utility: no Dock icon, lives in the menu bar.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let node = tauri::async_runtime::block_on(Node::start(node_config(app)?))?;
            forward_changes(app.handle().clone(), node.clone());
            app.manage(node);

            build_tray(app)?;
            if let Some(window) = app.get_webview_window("main") {
                window::place_bubble(&window)?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Quickie");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            app.state::<Node>().shutdown();
        }
    });
}
