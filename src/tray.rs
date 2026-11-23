use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DestroyMenu,
    GetCursorPos, LoadIconW, LoadImageW, RegisterClassW, SetForegroundWindow,
    TrackPopupMenu, HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED,
    MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON,
    WM_APP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

pub const WM_TRAYICON: u32 = WM_APP + 1;
pub const ID_OPEN_WEB_UI: usize = 2001;
pub const ID_PAUSE_RESUME: usize = 2002;
pub const ID_STOP: usize = 2003;
pub const ID_QUIT: usize = 2004;

pub struct TrayIcon {
    #[allow(dead_code)]
    pub hwnd: HWND,
    nid: NOTIFYICONDATAW,
}

unsafe impl Send for TrayIcon {}
unsafe impl Sync for TrayIcon {}

impl TrayIcon {
    pub fn new(hwnd: HWND, tooltip: &str) -> Result<Self, String> {
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;

        unsafe {
            let hicon = load_app_icon()?;
            nid.hIcon = hicon;

            let tip_encoded: Vec<u16> = tooltip.encode_utf16().chain(std::iter::once(0)).collect();
            let len = tip_encoded.len().min(nid.szTip.len());
            nid.szTip[..len].copy_from_slice(&tip_encoded[..len]);

            // Try default size first, then fallback to V3 / V2 sizes
            let mut added = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
            if !added {
                nid.cbSize = 952; // NOTIFYICONDATAW_V3_SIZE
                added = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
            }
            if !added {
                nid.cbSize = 936; // NOTIFYICONDATAW_V2_SIZE
                added = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
            }

            if !added {
                let err = GetLastError();
                return Err(format!("Failed to create system tray icon (error code: {:?})", err));
            }
        }

        Ok(Self { hwnd, nid })
    }

    pub fn set_tooltip(&mut self, text: &str) {
        let tip_encoded: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let len = tip_encoded.len().min(self.nid.szTip.len());
        self.nid.szTip = [0; 128];
        self.nid.szTip[..len].copy_from_slice(&tip_encoded[..len]);
        self.nid.uFlags = NIF_TIP;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.nid);
        }
    }

    pub fn show_notification(&mut self, title: &str, msg: &str, is_error: bool) {
        self.nid.uFlags = NIF_INFO;
        self.nid.dwInfoFlags = if is_error { NIIF_ERROR } else { NIIF_INFO };

        let title_encoded: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let len_t = title_encoded.len().min(self.nid.szInfoTitle.len());
        self.nid.szInfoTitle = [0; 64];
        self.nid.szInfoTitle[..len_t].copy_from_slice(&title_encoded[..len_t]);

        let msg_encoded: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
        let len_m = msg_encoded.len().min(self.nid.szInfo.len());
        self.nid.szInfo = [0; 256];
        self.nid.szInfo[..len_m].copy_from_slice(&msg_encoded[..len_m]);

        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.nid);
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
        }
    }
}

pub fn check_single_instance() -> Result<(), String> {
    unsafe {
        let _ = CreateMutexW(None, true, w!("Global\\KokoroTtsTrayAppMutex"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Err("Another instance of Kokoro TTS is already running.".to_string());
        }
    }
    Ok(())
}

pub fn create_hidden_window(wndproc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT) -> Result<HWND, String> {
    unsafe {
        let h_instance: HINSTANCE = GetModuleHandleW(None).map_err(|e| e.to_string())?.into();
        let class_name = w!("KokoroTtsMsgWndClass");

        let hicon = load_app_icon().unwrap_or_default();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: h_instance,
            hIcon: hicon,
            lpszClassName: class_name,
            ..Default::default()
        };

        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            Default::default(),
            class_name,
            w!("Kokoro TTS Message Window"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            0,
            0,
            None,
            None,
            h_instance,
            None,
        ).map_err(|e| format!("CreateWindowExW failed: {}", e))?;

        Ok(hwnd)
    }
}

pub fn show_tray_menu(hwnd: HWND, is_speaking: bool) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);

        let hmenu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        let _ = AppendMenuW(hmenu, MF_STRING, ID_OPEN_WEB_UI, w!("Open Web UI"));
        let pause_label = if is_speaking { w!("Pause / Resume") } else { w!("Resume") };
        let _ = AppendMenuW(hmenu, MF_STRING, ID_PAUSE_RESUME, pause_label);
        let _ = AppendMenuW(hmenu, MF_STRING, ID_STOP, w!("Stop"));
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(hmenu, MF_STRING, ID_QUIT, w!("Quit"));

        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(hmenu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
        let _ = DestroyMenu(hmenu);
    }
}

pub fn load_app_icon() -> Result<HICON, String> {
    unsafe {
        // 1. Try to load custom icon embedded into the PE binary resources (resource ID 1 set by winres)
        if let Ok(h_module) = GetModuleHandleW(None) {
            let h_instance: HINSTANCE = h_module.into();
            if let Ok(handle) = LoadImageW(
                h_instance,
                PCWSTR(1 as *const u16),
                IMAGE_ICON,
                0,
                0,
                LR_DEFAULTSIZE | LR_SHARED,
            ) {
                if !handle.is_invalid() {
                    return Ok(HICON(handle.0));
                }
            }

            if let Ok(hicon) = LoadIconW(h_instance, PCWSTR(1 as *const u16)) {
                if !hicon.is_invalid() {
                    return Ok(hicon);
                }
            }
        }

        // 2. Fallback to default application icon
        LoadIconW(None, IDI_APPLICATION).map_err(|e| e.to_string())
    }
}
