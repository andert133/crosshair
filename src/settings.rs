use serde::{Deserialize, Serialize};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub enabled: bool,
    pub color: String,
    pub line_width: i32,
    pub line_length: i32,
    pub gap: i32,
    pub dot_enabled: bool,
    pub dot_size: i32,
    pub hotkey_ctrl: bool,
    pub hotkey_alt: bool,
    pub hotkey_shift: bool,
    pub hotkey_win: bool,
    pub hotkey_key: String,
    pub autostart: bool,
    pub offset_x: i32,
    pub offset_y: i32,
    #[serde(default)]
    pub outline_enabled: bool,
    #[serde(default = "default_draw_line")]
    pub draw_left: bool,
    #[serde(default = "default_draw_line")]
    pub draw_right: bool,
    #[serde(default = "default_draw_line")]
    pub draw_top: bool,
    #[serde(default = "default_draw_line")]
    pub draw_bottom: bool,
}

fn default_draw_line() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            color: "#00ffff".to_string(), // Cyan
            line_width: 2,
            line_length: 8,
            gap: 4,
            dot_enabled: false,
            dot_size: 3,
            hotkey_ctrl: true,
            hotkey_alt: true,
            hotkey_shift: false,
            hotkey_win: false,
            hotkey_key: "C".to_string(),
            autostart: false,
            offset_x: 0,
            offset_y: 0,
            outline_enabled: false,
            draw_left: true,
            draw_right: true,
            draw_top: true,
            draw_bottom: true,
        }
    }
}

fn get_config_path() -> PathBuf {
    let mut path = env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    path.push("RustCrosshair");
    path
}

fn get_config_file() -> PathBuf {
    let mut path = get_config_path();
    path.push("settings.json");
    path
}

pub fn load_settings() -> Settings {
    let file_path = get_config_file();
    if file_path.exists() {
        if let Ok(content) = fs::read_to_string(file_path) {
            if let Ok(settings) = serde_json::from_str::<Settings>(&content) {
                return settings;
            }
        }
    }
    Settings::default()
}

pub fn save_settings(settings: &Settings) -> Result<(), String> {
    let dir_path = get_config_path();
    if !dir_path.exists() {
        fs::create_dir_all(&dir_path).map_err(|e| e.to_string())?;
    }
    let file_path = get_config_file();
    let content = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(file_path, content).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let exe_path = env::current_exe().map_err(|e| e.to_string())?;
    let path_str = exe_path.to_string_lossy().to_string();

    let subkey: Vec<u16> = OsStr::new("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let val_name: Vec<u16> = OsStr::new("RustCrosshair")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut hkey = 0;
    unsafe {
        let res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        );
        if res != 0 {
            return Err(format!("RegOpenKeyExW failed with error {}", res));
        }

        if enabled {
            let path_u16: Vec<u16> = OsStr::new(&path_str)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let res = RegSetValueExW(
                hkey,
                val_name.as_ptr(),
                0,
                REG_SZ,
                path_u16.as_ptr() as *const u8,
                (path_u16.len() * 2) as u32,
            );
            RegCloseKey(hkey);
            if res != 0 {
                return Err(format!("RegSetValueExW failed with error {}", res));
            }
        } else {
            let res = RegDeleteValueW(hkey, val_name.as_ptr());
            RegCloseKey(hkey);
            // Ignore ERROR_FILE_NOT_FOUND (2)
            if res != 0 && res != 2 {
                return Err(format!("RegDeleteValueW failed with error {}", res));
            }
        }
    }
    Ok(())
}

pub fn get_hotkey_modifiers(settings: &Settings) -> u32 {
    let mut mods = 0;
    if settings.hotkey_alt {
        mods |= 0x0001; // MOD_ALT
    }
    if settings.hotkey_ctrl {
        mods |= 0x0002; // MOD_CONTROL
    }
    if settings.hotkey_shift {
        mods |= 0x0004; // MOD_SHIFT
    }
    if settings.hotkey_win {
        mods |= 0x0008; // MOD_WIN
    }
    mods
}

pub fn parse_virtual_key(key_str: &str) -> u32 {
    let key_upper = key_str.to_uppercase();
    match key_upper.as_str() {
        "SPACE" => 0x20,   // VK_SPACE
        "INSERT" => 0x2D,  // VK_INSERT
        "DELETE" => 0x2E,  // VK_DELETE
        "HOME" => 0x24,    // VK_HOME
        "END" => 0x23,     // VK_END
        "PAGEUP" => 0x21,   // VK_PRIOR
        "PAGEDOWN" => 0x22, // VK_NEXT
        "F1" => 0x70,
        "F2" => 0x71,
        "F3" => 0x72,
        "F4" => 0x73,
        "F5" => 0x74,
        "F6" => 0x75,
        "F7" => 0x76,
        "F8" => 0x77,
        "F9" => 0x78,
        "F10" => 0x79,
        "F11" => 0x7A,
        "F12" => 0x7B,
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
            if ch >= 'A' && ch <= 'Z' {
                ch as u32
            } else if ch >= '0' && ch <= '9' {
                ch as u32
            } else {
                0x43 // Default to 'C'
            }
        }
        _ => 0x43, // Default to 'C'
    }
}
