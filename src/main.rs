#![windows_subsystem = "windows"]

mod audio;
mod autostart;
mod clean;
mod clipboard;
mod hotkey;
mod playback;
mod settings;
mod tray;
mod tts;
mod web;

use clipboard::capture_selected_text;
use hotkey::{HotkeyManager, HOTKEY_ID};
use playback::{PlaybackCoordinator, PlayStatus};
use settings::AppSettings;
use std::sync::Arc;
use std::thread;
use tray::{
    check_single_instance, create_hidden_window, show_tray_menu, TrayIcon, ID_OPEN_WEB_UI,
    ID_PAUSE_RESUME, ID_QUIT, ID_STOP, WM_TRAYICON,
};
use tts::kokoro::KokoroEngine;
use web::{start_web_server, WebState};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage, TranslateMessage, MSG,
    WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_RBUTTONUP,
};

static TRAY_REF: parking_lot::Mutex<Option<TrayIcon>> = parking_lot::Mutex::new(None);
static SETTINGS_REF: parking_lot::Mutex<Option<Arc<parking_lot::Mutex<AppSettings>>>> =
    parking_lot::Mutex::new(None);
static COORDINATOR_REF: parking_lot::Mutex<Option<Arc<PlaybackCoordinator>>> =
    parking_lot::Mutex::new(None);
static HOTKEY_MGR_REF: parking_lot::Mutex<Option<HotkeyManager>> = parking_lot::Mutex::new(None);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            if wparam.0 == HOTKEY_ID as usize {
                handle_hotkey_press();
            }
            LRESULT(0)
        }
        WM_TRAYICON => {
            let event = (lparam.0 & 0xffff) as u32;
            match event {
                WM_RBUTTONUP => {
                    let speaking = is_currently_speaking();
                    show_tray_menu(hwnd, speaking);
                }
                WM_LBUTTONDBLCLK => {
                    open_web_ui();
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd_id = wparam.0 as usize;
            match cmd_id {
                ID_OPEN_WEB_UI => open_web_ui(),
                ID_PAUSE_RESUME => toggle_pause_resume(),
                ID_STOP => stop_playback(),
                ID_QUIT => {
                    PostQuitMessage(0);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn is_currently_speaking() -> bool {
    let guard = COORDINATOR_REF.lock();
    if let Some(ref coord) = *guard {
        coord.get_state().status == PlayStatus::Playing
    } else {
        false
    }
}

fn handle_hotkey_press() {
    let coord = {
        let guard = COORDINATOR_REF.lock();
        guard.clone()
    };

    let coord = match coord {
        Some(c) => c,
        None => {
            notify("Model Loading", "Kokoro TTS is still initializing or downloading model.", true);
            return;
        }
    };

    let state = coord.get_state();
    if state.status == PlayStatus::Playing {
        // Pressing hotkey while speaking stops playback
        coord.stop();
        if let Some(tray) = TRAY_REF.lock().as_mut() {
            tray.set_tooltip("Kokoro TTS (Idle)");
        }
        return;
    }

    let (mode, dont_restore, voice, speed, clean) = {
        let guard = SETTINGS_REF.lock();
        if let Some(ref s_arc) = *guard {
            let s = s_arc.lock();
            (
                s.clipboard_mode,
                s.dont_restore_clipboard,
                s.voice.clone(),
                s.speed,
                s.clean_text,
            )
        } else {
            (
                settings::ClipboardMode::AutoCopy,
                false,
                "af_bella".to_string(),
                1.0,
                true,
            )
        }
    };

    match capture_selected_text(mode, dont_restore) {
        Ok(text) => {
            let text_to_play = if clean {
                clean::clean_text(&text)
            } else {
                text
            };

            if text_to_play.trim().is_empty() {
                notify("Notice", "No readable text found after cleanup.", false);
                return;
            }

            let count = text_to_play.chars().count();
            notify("Reading Aloud", &format!("Reading {} characters with Kokoro...", count), false);
            if let Some(tray) = TRAY_REF.lock().as_mut() {
                tray.set_tooltip("Kokoro TTS (Speaking...)");
            }
            // Dispatches to coordinator: chunks text, starts synthesis lookahead, streams to audio
            coord.play_text(&text_to_play, &voice, speed);
        }
        Err(err) => {
            notify("Notice", &err, true);
        }
    }
}

fn toggle_pause_resume() {
    let guard = COORDINATOR_REF.lock();
    if let Some(ref coord) = *guard {
        let state = coord.get_state();
        if state.status == PlayStatus::Playing {
            coord.pause();
            if let Some(tray) = TRAY_REF.lock().as_mut() {
                tray.set_tooltip("Kokoro TTS (Paused)");
            }
        } else if state.status == PlayStatus::Paused {
            coord.resume();
            if let Some(tray) = TRAY_REF.lock().as_mut() {
                tray.set_tooltip("Kokoro TTS (Speaking...)");
            }
        }
    }
}

fn stop_playback() {
    let guard = COORDINATOR_REF.lock();
    if let Some(ref coord) = *guard {
        coord.stop();
        if let Some(tray) = TRAY_REF.lock().as_mut() {
            tray.set_tooltip("Kokoro TTS (Idle)");
            tray.show_notification("Playback Stopped", "Speech playback has been stopped.", false);
        }
    }
}

fn open_web_ui() {
    let port = {
        let guard = SETTINGS_REF.lock();
        if let Some(ref s_arc) = *guard {
            s_arc.lock().port
        } else {
            8787
        }
    };
    let url = format!("http://127.0.0.1:{}", port);
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", &url])
        .spawn();
}

fn notify(title: &str, msg: &str, is_error: bool) {
    if let Some(tray) = TRAY_REF.lock().as_mut() {
        tray.show_notification(title, msg, is_error);
    }
}

fn main() {
    // 1. Single Instance Check
    if let Err(msg) = check_single_instance() {
        eprintln!("{}", msg);
        return;
    }

    // 2. Load Settings
    let settings = AppSettings::load();
    let hotkey_str = settings.hotkey.clone();
    let port = settings.port;
    let settings_arc = Arc::new(parking_lot::Mutex::new(settings));
    *SETTINGS_REF.lock() = Some(settings_arc.clone());

    // 3. Create Hidden Window for Message Handling
    let hwnd = match create_hidden_window(wndproc) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to create window: {}", e);
            return;
        }
    };

    // 4. Create System Tray Icon
    let mut tray_opt = match TrayIcon::new(hwnd, "Kokoro TTS (Initializing...)") {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("Warning: could not create tray icon: {}", e);
            None
        }
    };

    // 5. Register Global Hotkey
    let mut hotkey_mgr = HotkeyManager::new(hwnd);
    if let Err(err) = hotkey_mgr.register(&hotkey_str) {
        if let Some(ref mut tray) = tray_opt {
            tray.show_notification("Hotkey Conflict", &err, true);
        }
    } else {
        if let Some(ref mut tray) = tray_opt {
            tray.set_tooltip(&format!("Kokoro TTS (Hotkey: {})", hotkey_str));
        }
    }
    *HOTKEY_MGR_REF.lock() = Some(hotkey_mgr);

    *TRAY_REF.lock() = tray_opt;

    // 6. Spawn Web Server Thread Immediately on Startup
    let settings_clone = settings_arc.clone();
    let hotkey_updater = Arc::new(move |new_hotkey: &str| -> Result<(), String> {
        let mut mgr_guard = HOTKEY_MGR_REF.lock();
        if let Some(ref mut mgr) = *mgr_guard {
            mgr.register(new_hotkey)?;
            if let Some(t) = TRAY_REF.lock().as_mut() {
                t.set_tooltip(&format!("Kokoro TTS (Hotkey: {})", new_hotkey));
            }
        }
        Ok(())
    });

    let web_coord = Arc::new(parking_lot::Mutex::new(None));
    let web_coord_clone = web_coord.clone();

    let web_state = WebState {
        coordinator: web_coord,
        settings: settings_clone,
        hotkey_updater,
    };

    thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to create tokio runtime: {}", e);
                return;
            }
        };

        rt.block_on(async move {
            if let Err(e) = start_web_server(web_state, port).await {
                eprintln!("Web server error: {}", e);
            }
        });
    });

    // 7. Spawn Background Model Loading & Coordinator Setup Thread
    thread::spawn(move || {
        // Download models if not already present
        let model_dir = match KokoroEngine::download_models_if_needed(|p, msg| {
            if let Some(t) = TRAY_REF.lock().as_mut() {
                t.set_tooltip(&format!("Kokoro TTS ({:.0}%): {}", p * 100.0, msg));
            }
        }) {
            Ok(dir) => dir,
            Err(e) => {
                notify("Model Download Error", &e, true);
                return;
            }
        };

        if let Some(t) = TRAY_REF.lock().as_mut() {
            t.set_tooltip("Kokoro TTS: Loading engine into memory...");
        }

        // Initialize Kokoro Engine (limits threads to 4, warms up, below-normal priority)
        let engine = match KokoroEngine::new(&model_dir) {
            Ok(eng) => Arc::new(eng),
            Err(e) => {
                notify("Engine Error", &format!("Failed to load Kokoro ONNX model: {}", e), true);
                return;
            }
        };

        // Initialize Playback Coordinator
        let coordinator = match PlaybackCoordinator::new(engine) {
            Ok(c) => c,
            Err(e) => {
                notify("Audio Error", &format!("Failed to initialize audio output: {}", e), true);
                return;
            }
        };

        *COORDINATOR_REF.lock() = Some(coordinator.clone());
        *web_coord_clone.lock() = Some(coordinator);

        if let Some(t) = TRAY_REF.lock().as_mut() {
            t.set_tooltip("Kokoro TTS (Ready)");
            t.show_notification(
                "Kokoro TTS Ready",
                &format!("Engine loaded. Press {} to read selected text.", hotkey_str),
                false,
            );
        }
    });

    // 8. Main Windows Message Loop
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
