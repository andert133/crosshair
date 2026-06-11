use std::os::windows::ffi::OsStrExt;
use std::sync::{Arc, RwLock};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, FillRect, GetDC, LineTo, MoveToEx, ReleaseDC, SelectObject, PAINTSTRUCT, PS_SOLID,
    MonitorFromPoint, MonitorFromWindow, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTOPRIMARY
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, GetClientRect, GetCursorPos, PostQuitMessage, RegisterClassW, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, TrackPopupMenu, GWLP_USERDATA, HICON, ICONINFO, LWA_COLORKEY, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, SWP_NOACTIVATE, SWP_NOSIZE, SW_HIDE, SW_SHOWNA, TPM_RETURNCMD, TPM_RIGHTBUTTON, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP
};

use crate::server::{WM_EXIT_APPLICATION, WM_TOGGLE_VISIBILITY, WM_UPDATE_SETTINGS};
use crate::settings::{get_hotkey_modifiers, parse_virtual_key, save_settings, set_autostart, Settings};

pub const OVERLAY_WINDOW_SIZE: i32 = 200;
pub const WM_TRAYICON: u32 = 0x0400 + 10; // WM_USER + 10

pub struct AppState {
    pub settings: Arc<RwLock<Settings>>,
}

// Helper to parse "#rrggbb" hex string to GDI COLORREF (0x00BBGGRR)
pub fn parse_hex_color(hex: &str) -> u32 {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return 0x00FFFF00; // Default: Cyan
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);

    // Prevent pure black conflict with transparency color key
    let (r, g, b) = if r == 0 && g == 0 && b == 0 {
        (1, 1, 1)
    } else {
        (r, g, b)
    };

    // 0x00BBGGRR format
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

// Dynamically draws a 16x16 HICON in memory representing a crosshair
unsafe fn create_custom_tray_icon() -> HICON {
    let hdc_screen = GetDC(0);
    let hdc_mem = CreateCompatibleDC(hdc_screen);
    let hbm_color = CreateCompatibleBitmap(hdc_screen, 16, 16);
    let hbm_mask = CreateCompatibleBitmap(hdc_mem, 16, 16); // Monochrome by default

    // 1. Draw color bitmap (black background, cyan cross)
    let old_color = SelectObject(hdc_mem, hbm_color as _);
    let bg_brush = CreateSolidBrush(0); // Black
    let rect = RECT { left: 0, top: 0, right: 16, bottom: 16 };
    FillRect(hdc_mem, &rect, bg_brush);
    DeleteObject(bg_brush);

    let cyan_pen = CreatePen(PS_SOLID, 2, 0x00FFFF00); // Cyan (0x00BBGGRR)
    let old_pen = SelectObject(hdc_mem, cyan_pen as _);

    // Draw Crosshair lines
    MoveToEx(hdc_mem, 8, 2, std::ptr::null_mut());
    LineTo(hdc_mem, 8, 14);
    MoveToEx(hdc_mem, 2, 8, std::ptr::null_mut());
    LineTo(hdc_mem, 14, 8);

    SelectObject(hdc_mem, old_pen);
    DeleteObject(cyan_pen);

    // 2. Draw mask bitmap (white background/transparent, black lines/opaque)
    SelectObject(hdc_mem, hbm_mask as _);
    let white_brush = CreateSolidBrush(0x00FFFFFF); // White
    FillRect(hdc_mem, &rect, white_brush);
    DeleteObject(white_brush);

    let black_pen = CreatePen(PS_SOLID, 2, 0); // Black
    let old_pen = SelectObject(hdc_mem, black_pen as _);

    // Draw Crosshair lines on mask
    MoveToEx(hdc_mem, 8, 2, std::ptr::null_mut());
    LineTo(hdc_mem, 8, 14);
    MoveToEx(hdc_mem, 2, 8, std::ptr::null_mut());
    LineTo(hdc_mem, 14, 8);

    SelectObject(hdc_mem, old_pen);
    DeleteObject(black_pen);

    // Clean up
    SelectObject(hdc_mem, old_color);
    DeleteDC(hdc_mem);
    ReleaseDC(0, hdc_screen);

    let icon_info = ICONINFO {
        fIcon: 1, // TRUE
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbm_mask,
        hbmColor: hbm_color,
    };

    let hicon = CreateIconIndirect(&icon_info);

    DeleteObject(hbm_mask);
    DeleteObject(hbm_color);

    hicon
}

unsafe fn add_tray_icon(hwnd: HWND, hicon: HICON) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;
    nid.hIcon = hicon;

    // Tooltip wide string
    let tip = std::ffi::OsStr::new("Прицел (Rust)\nНажмите для настроек")
        .encode_wide()
        .collect::<Vec<u16>>();
    let len = tip.len().min(nid.szTip.len() - 1);
    std::ptr::copy_nonoverlapping(tip.as_ptr(), nid.szTip.as_mut_ptr(), len);
    nid.szTip[len] = 0;

    Shell_NotifyIconW(NIM_ADD, &nid);
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn register_current_hotkey(hwnd: HWND, settings: &Settings) {
    UnregisterHotKey(hwnd, 1);
    let mods = get_hotkey_modifiers(settings);
    let vk = parse_virtual_key(&settings.hotkey_key);
    if vk != 0 {
        RegisterHotKey(hwnd, 1, mods, vk);
    }
}

pub fn create_overlay_window(state_ptr: *mut AppState) -> HWND {
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class_name: Vec<u16> = std::ffi::OsStr::new("CrosshairOverlayClass")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(overlay_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: 0,
        hCursor: 0,
        hbrBackground: unsafe { CreateSolidBrush(0) }, // Black background
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };

    unsafe {
        RegisterClassW(&wc);
    }

    let (offset_x, offset_y) = {
        let state = unsafe { &*state_ptr };
        let settings = state.settings.read().unwrap();
        (settings.offset_x, settings.offset_y)
    };

    // Get monitor info based on mouse cursor position to support multi-monitor and custom scaling setups
    let mut cursor_pt = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut cursor_pt);
    }

    let monitor = unsafe {
        MonitorFromPoint(
            cursor_pt,
            MONITOR_DEFAULTTOPRIMARY,
        )
    };

    let mut monitor_info: MONITORINFO = unsafe { std::mem::zeroed() };
    monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    unsafe {
        GetMonitorInfoW(monitor, &mut monitor_info as *mut _ as *mut _);
    }

    let monitor_width = monitor_info.rcMonitor.right - monitor_info.rcMonitor.left;
    let monitor_height = monitor_info.rcMonitor.bottom - monitor_info.rcMonitor.top;

    let x = monitor_info.rcMonitor.left + (monitor_width - OVERLAY_WINDOW_SIZE) / 2 + offset_x;
    let y = monitor_info.rcMonitor.top + (monitor_height - OVERLAY_WINDOW_SIZE) / 2 + offset_y;

    let dw_ex_style = WS_EX_LAYERED
        | WS_EX_TRANSPARENT
        | WS_EX_TOPMOST
        | WS_EX_NOACTIVATE
        | WS_EX_TOOLWINDOW;
    let dw_style = WS_POPUP;

    let hwnd = unsafe {
        CreateWindowExW(
            dw_ex_style,
            class_name.as_ptr(),
            std::ptr::null(),
            dw_style,
            x,
            y,
            OVERLAY_WINDOW_SIZE,
            OVERLAY_WINDOW_SIZE,
            0,
            0,
            instance,
            state_ptr as _,
        )
    };

    if hwnd == 0 {
        panic!("Failed to create overlay window");
    }

    unsafe {
        SetLayeredWindowAttributes(hwnd, 0, 0, LWA_COLORKEY);
    }

    hwnd
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        windows_sys::Win32::UI::WindowsAndMessaging::WM_CREATE => {
            let create_struct = lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            let state_ptr = (*create_struct).lpCreateParams as *mut AppState;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);

            let state = &*state_ptr;
            let settings = state.settings.read().unwrap();

            // Set up hotkeys
            register_current_hotkey(hwnd, &*settings);

            // Set up system tray icon
            let hicon = create_custom_tray_icon();
            add_tray_icon(hwnd, hicon);

            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_PAINT => {
            let state_ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);
            if state_ptr == 0 {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }

            let state = &*(state_ptr as *const AppState);
            let settings = state.settings.read().unwrap();

            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rect);

            let bg_brush = CreateSolidBrush(0);
            FillRect(hdc, &rect, bg_brush);
            DeleteObject(bg_brush);

            if settings.enabled {
                let color_val = parse_hex_color(&settings.color);
                let brush = CreateSolidBrush(color_val);
                let outline_brush = CreateSolidBrush(0x00010101); // Near-black outline brush (avoids ColorKey transparency)

                let cx = (rect.right - rect.left) / 2;
                let cy = (rect.bottom - rect.top) / 2;
                let len = settings.line_length;
                let gap = settings.gap;
                let line_width = settings.line_width;

                let vl_left = cx - (line_width / 2);
                let vl_right = vl_left + line_width;
                let hl_top = cy - (line_width / 2);
                let hl_bottom = hl_top + line_width;

                // Horizontal Left
                if settings.draw_left {
                    let rect_left = RECT {
                        left: vl_left - gap - len + 1,
                        top: hl_top,
                        right: vl_left - gap + 1,
                        bottom: hl_bottom,
                    };
                    if settings.outline_enabled {
                        let out_rect = RECT {
                            left: rect_left.left - 1,
                            top: rect_left.top - 1,
                            right: rect_left.right + 1,
                            bottom: rect_left.bottom + 1,
                        };
                        FillRect(hdc, &out_rect, outline_brush);
                    }
                    FillRect(hdc, &rect_left, brush);
                }

                // Horizontal Right
                if settings.draw_right {
                    let rect_right = RECT {
                        left: vl_right + gap - 1,
                        top: hl_top,
                        right: vl_right + gap - 1 + len,
                        bottom: hl_bottom,
                    };
                    if settings.outline_enabled {
                        let out_rect = RECT {
                            left: rect_right.left - 1,
                            top: rect_right.top - 1,
                            right: rect_right.right + 1,
                            bottom: rect_right.bottom + 1,
                        };
                        FillRect(hdc, &out_rect, outline_brush);
                    }
                    FillRect(hdc, &rect_right, brush);
                }

                // Vertical Top
                if settings.draw_top {
                    let rect_top = RECT {
                        left: vl_left,
                        top: hl_top - gap - len + 1,
                        right: vl_right,
                        bottom: hl_top - gap + 1,
                    };
                    if settings.outline_enabled {
                        let out_rect = RECT {
                            left: rect_top.left - 1,
                            top: rect_top.top - 1,
                            right: rect_top.right + 1,
                            bottom: rect_top.bottom + 1,
                        };
                        FillRect(hdc, &out_rect, outline_brush);
                    }
                    FillRect(hdc, &rect_top, brush);
                }

                // Vertical Bottom
                if settings.draw_bottom {
                    let rect_bottom = RECT {
                        left: vl_left,
                        top: hl_bottom + gap - 1,
                        right: vl_right,
                        bottom: hl_bottom + gap - 1 + len,
                    };
                    if settings.outline_enabled {
                        let out_rect = RECT {
                            left: rect_bottom.left - 1,
                            top: rect_bottom.top - 1,
                            right: rect_bottom.right + 1,
                            bottom: rect_bottom.bottom + 1,
                        };
                        FillRect(hdc, &out_rect, outline_brush);
                    }
                    FillRect(hdc, &rect_bottom, brush);
                }

                if settings.dot_enabled && settings.dot_size > 0 {
                    let dot_size = settings.dot_size;
                    let rect_dot = RECT {
                        left: cx - (dot_size / 2),
                        top: cy - (dot_size / 2),
                        right: cx - (dot_size / 2) + dot_size,
                        bottom: cy - (dot_size / 2) + dot_size,
                    };
                    if settings.outline_enabled {
                        let out_rect = RECT {
                            left: rect_dot.left - 1,
                            top: rect_dot.top - 1,
                            right: rect_dot.right + 1,
                            bottom: rect_dot.bottom + 1,
                        };
                        FillRect(hdc, &out_rect, outline_brush);
                    }
                    FillRect(hdc, &rect_dot, brush);
                }

                DeleteObject(brush);
                DeleteObject(outline_brush);
            }

            EndPaint(hwnd, &ps);
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_HOTKEY => {
            let state_ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);
            if state_ptr != 0 {
                let state = &*(state_ptr as *const AppState);
                let mut settings = state.settings.write().unwrap();
                settings.enabled = !settings.enabled;
                let _ = save_settings(&*settings);

                // Toggle visibility in UI thread
                if settings.enabled {
                    windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_SHOWNA);
                } else {
                    windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_HIDE);
                }
                windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1);
            }
            0
        }
        WM_TRAYICON => {
            let event = lparam as u32;
            if event == windows_sys::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP {
                // Show Tray Context Menu
                let state_ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);
                if state_ptr != 0 {
                    let state = &*(state_ptr as *const AppState);
                    let (enabled, autostart) = {
                        let settings = state.settings.read().unwrap();
                        (settings.enabled, settings.autostart)
                    };

                    let mut pt = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);

                    // Required to focus on the menu and close it if clicked outside
                    windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);

                    let menu = CreatePopupMenu();

                    // 1. Open Settings Title
                    let item_settings = std::ffi::OsStr::new("Настройки (в браузере)")
                        .encode_wide()
                        .chain(std::iter::once(0))
                        .collect::<Vec<u16>>();
                    AppendMenuW(menu, MF_STRING, 101, item_settings.as_ptr());

                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

                    // 2. Toggle Visibility
                    let item_toggle = std::ffi::OsStr::new("Включить прицел")
                        .encode_wide()
                        .chain(std::iter::once(0))
                        .collect::<Vec<u16>>();
                    AppendMenuW(
                        menu,
                        MF_STRING | if enabled { MF_CHECKED } else { MF_UNCHECKED },
                        102,
                        item_toggle.as_ptr(),
                    );

                    // 3. Toggle Autostart
                    let item_auto = std::ffi::OsStr::new("Запускать при старте Windows")
                        .encode_wide()
                        .chain(std::iter::once(0))
                        .collect::<Vec<u16>>();
                    AppendMenuW(
                        menu,
                        MF_STRING | if autostart { MF_CHECKED } else { MF_UNCHECKED },
                        103,
                        item_auto.as_ptr(),
                    );

                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

                    // 4. Exit
                    let item_exit = std::ffi::OsStr::new("Выход")
                        .encode_wide()
                        .chain(std::iter::once(0))
                        .collect::<Vec<u16>>();
                    AppendMenuW(menu, MF_STRING, 104, item_exit.as_ptr());

                    // Track menu clicks
                    let selection = TrackPopupMenu(
                        menu,
                        TPM_RIGHTBUTTON | TPM_RETURNCMD,
                        pt.x,
                        pt.y,
                        0,
                        hwnd,
                        std::ptr::null(),
                    );

                    DestroyMenu(menu);

                    match selection {
                        101 => {
                            // Open Settings
                            let url = std::ffi::OsStr::new("http://127.0.0.1:25432")
                                .encode_wide()
                                .chain(std::iter::once(0))
                                .collect::<Vec<u16>>();
                            ShellExecuteW(0, std::ptr::null(), url.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
                        }
                        102 => {
                            // Toggle enabled state
                            let mut settings = state.settings.write().unwrap();
                            settings.enabled = !settings.enabled;
                            let _ = save_settings(&*settings);

                            if settings.enabled {
                                windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_SHOWNA);
                            } else {
                                windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_HIDE);
                            }
                            windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1);
                        }
                        103 => {
                            // Toggle autostart
                            let mut settings = state.settings.write().unwrap();
                            settings.autostart = !settings.autostart;
                            let _ = save_settings(&*settings);
                            let _ = set_autostart(settings.autostart);
                        }
                        104 => {
                            // Exit
                            DestroyWindow(hwnd);
                        }
                        _ => {}
                    }
                }
            } else if event == windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP {
                // Left click -> Open Web configuration URL
                let url = std::ffi::OsStr::new("http://127.0.0.1:25432")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect::<Vec<u16>>();
                ShellExecuteW(0, std::ptr::null(), url.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
            }
            0
        }
        WM_UPDATE_SETTINGS => {
            let state_ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);
            if state_ptr != 0 {
                let state = &*(state_ptr as *const AppState);
                let (enabled, offset_x, offset_y) = {
                    let settings = state.settings.read().unwrap();
                    register_current_hotkey(hwnd, &*settings); // Re-bind hotkey on config change
                    (settings.enabled, settings.offset_x, settings.offset_y)
                };

                // Get monitor info for the active window to support multi-monitor setups
                let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
                let mut monitor_info: MONITORINFO = std::mem::zeroed();
                monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                GetMonitorInfoW(monitor, &mut monitor_info as *mut _ as *mut _);

                let monitor_width = monitor_info.rcMonitor.right - monitor_info.rcMonitor.left;
                let monitor_height = monitor_info.rcMonitor.bottom - monitor_info.rcMonitor.top;

                let x = monitor_info.rcMonitor.left + (monitor_width - OVERLAY_WINDOW_SIZE) / 2 + offset_x;
                let y = monitor_info.rcMonitor.top + (monitor_height - OVERLAY_WINDOW_SIZE) / 2 + offset_y;

                SetWindowPos(
                    hwnd,
                    0,
                    x,
                    y,
                    OVERLAY_WINDOW_SIZE,
                    OVERLAY_WINDOW_SIZE,
                    SWP_NOACTIVATE | SWP_NOSIZE,
                );

                if enabled {
                    windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_SHOWNA);
                } else {
                    windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_HIDE);
                }

                windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1);
            }
            0
        }
        WM_TOGGLE_VISIBILITY => {
            let state_ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);
            if state_ptr != 0 {
                let state = &*(state_ptr as *const AppState);
                let enabled = state.settings.read().unwrap().enabled;

                if enabled {
                    windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_SHOWNA);
                } else {
                    windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_HIDE);
                }

                windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1);
            }
            0
        }
        WM_EXIT_APPLICATION => {
            DestroyWindow(hwnd);
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
