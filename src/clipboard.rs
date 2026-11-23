use crate::settings::ClipboardMode;
use std::thread::sleep;
use std::time::{Duration, Instant};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_C, VK_CONTROL, VK_MENU,
};

pub fn capture_selected_text(mode: ClipboardMode, dont_restore: bool) -> Result<String, String> {
    match mode {
        ClipboardMode::DirectRead => read_clipboard_direct(),
        ClipboardMode::AutoCopy => auto_copy_and_restore(dont_restore),
    }
}

fn read_clipboard_direct() -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("Clipboard error: {}", e))?;
    let text = clipboard.get_text().map_err(|_| "No text found in clipboard".to_string())?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Clipboard is empty".to_string());
    }
    Ok(trimmed.to_string())
}

fn auto_copy_and_restore(dont_restore: bool) -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("Clipboard init failed: {}", e))?;
    let backup_text: Option<String> = clipboard.get_text().ok();

    // Release Alt modifier if user was holding it during hotkey press
    send_key(VK_MENU.0, KEYEVENTF_KEYUP);
    sleep(Duration::from_millis(15));

    // Simulate Ctrl+C
    simulate_ctrl_c();

    // Poll clipboard up to 300ms for text
    let start = Instant::now();
    let timeout = Duration::from_millis(300);
    let mut captured_text = None;

    while start.elapsed() < timeout {
        sleep(Duration::from_millis(25));
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(text) = cb.get_text() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    // Check if it's different from backup, or if backup was None
                    if backup_text.as_deref() != Some(&text) {
                        captured_text = Some(trimmed.to_string());
                        break;
                    }
                }
            }
        }
    }

    // If still no distinct text, check if clipboard has valid non-empty text right now
    if captured_text.is_none() {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(text) = cb.get_text() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    captured_text = Some(trimmed.to_string());
                }
            }
        }
    }

    // Restore previous clipboard if requested
    if !dont_restore {
        if let Some(ref old_text) = backup_text {
            // Short delay to ensure the OS and target app have finished reading the copy
            sleep(Duration::from_millis(40));
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(old_text);
            }
        }
    }

    captured_text.ok_or_else(|| "No text selected".to_string())
}

fn send_key(vk: u16, flags: KEYBD_EVENT_FLAGS) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn simulate_ctrl_c() {
    // Key down: Ctrl
    send_key(VK_CONTROL.0, KEYBD_EVENT_FLAGS(0));
    sleep(Duration::from_millis(5));

    // Key down: C
    send_key(VK_C.0, KEYBD_EVENT_FLAGS(0));
    sleep(Duration::from_millis(15));

    // Key up: C
    send_key(VK_C.0, KEYEVENTF_KEYUP);
    sleep(Duration::from_millis(5));

    // Key up: Ctrl
    send_key(VK_CONTROL.0, KEYEVENTF_KEYUP);
}
