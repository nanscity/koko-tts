use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};

pub const HOTKEY_ID: i32 = 1001;

pub struct HotkeyManager {
    hwnd: HWND,
    current_hotkey: String,
}

unsafe impl Send for HotkeyManager {}
unsafe impl Sync for HotkeyManager {}

impl HotkeyManager {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            current_hotkey: String::new(),
        }
    }

    pub fn register(&mut self, hotkey_str: &str) -> Result<(), String> {
        let (mods, vk) = parse_hotkey(hotkey_str)?;

        unsafe {
            // Unregister prior hotkey if present
            let _ = UnregisterHotKey(self.hwnd, HOTKEY_ID);

            // Register new hotkey with MOD_NOREPEAT
            let res = RegisterHotKey(self.hwnd, HOTKEY_ID, mods | MOD_NOREPEAT, vk);
            if res.is_err() {
                return Err(format!("Hotkey conflict: '{}' is already in use by another application.", hotkey_str));
            }
        }

        self.current_hotkey = hotkey_str.to_string();
        Ok(())
    }

    pub fn unregister(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(self.hwnd, HOTKEY_ID);
        }
        self.current_hotkey.clear();
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister();
    }
}

pub fn parse_hotkey(s: &str) -> Result<(HOT_KEY_MODIFIERS, u32), String> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut key_code: Option<u32> = None;

    for part in s.split('+') {
        let trimmed = part.trim();
        match trimmed.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "windows" | "super" => mods |= MOD_WIN,
            other => {
                if key_code.is_some() {
                    return Err(format!("Multiple keys specified in hotkey: {}", s));
                }
                key_code = Some(parse_key(other)?);
            }
        }
    }

    let vk = key_code.ok_or_else(|| format!("No main key specified in hotkey: {}", s))?;
    Ok((mods, vk))
}

fn parse_key(k: &str) -> Result<u32, String> {
    let upper = k.to_uppercase();
    if upper.len() == 1 {
        let c = upper.chars().next().unwrap();
        if c.is_ascii_alphanumeric() {
            return Ok(c as u32);
        }
    }

    match upper.as_str() {
        "F1" => Ok(0x70),
        "F2" => Ok(0x71),
        "F3" => Ok(0x72),
        "F4" => Ok(0x73),
        "F5" => Ok(0x74),
        "F6" => Ok(0x75),
        "F7" => Ok(0x76),
        "F8" => Ok(0x77),
        "F9" => Ok(0x78),
        "F10" => Ok(0x79),
        "F11" => Ok(0x7A),
        "F12" => Ok(0x7B),
        "SPACE" => Ok(0x20),
        "RETURN" | "ENTER" => Ok(0x0D),
        "TAB" => Ok(0x09),
        "ESCAPE" | "ESC" => Ok(0x1B),
        _ => Err(format!("Unknown key: '{}'", k)),
    }
}
