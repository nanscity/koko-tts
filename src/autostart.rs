use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const APP_NAME: &str = "KokoroTTS";
const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

pub fn set_autostart(enable: bool) -> Result<(), String> {
    let app_name_u16: Vec<u16> = APP_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let app_name_pcwstr = PCWSTR(app_name_u16.as_ptr());

    unsafe {
        let mut hkey = Default::default();
        let res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            0,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            &mut hkey,
        );
        if res.is_err() {
            return Err("Failed to open Windows Run registry key".to_string());
        }

        if enable {
            let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
            let path_str = format!("\"{}\"", exe_path.display());
            let path_u16: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

            let set_res = RegSetValueExW(
                hkey,
                app_name_pcwstr,
                0,
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    path_u16.as_ptr() as *const u8,
                    path_u16.len() * 2,
                )),
            );
            let _ = RegCloseKey(hkey);
            if set_res.is_err() {
                return Err("Failed to write to Run registry key".to_string());
            }
        } else {
            let del_res = RegDeleteValueW(hkey, app_name_pcwstr);
            let _ = RegCloseKey(hkey);
            // Ignore if value didn't exist
            let _ = del_res;
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn is_autostart_enabled() -> bool {
    let app_name_u16: Vec<u16> = APP_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let app_name_pcwstr = PCWSTR(app_name_u16.as_ptr());

    unsafe {
        let mut hkey = Default::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_QUERY_VALUE, &mut hkey).is_err() {
            return false;
        }

        let query_res = RegQueryValueExW(hkey, app_name_pcwstr, None, None, None, None);
        let _ = RegCloseKey(hkey);
        query_res.is_ok()
    }
}
