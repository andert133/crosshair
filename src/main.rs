#![windows_subsystem = "windows"]

mod settings;
mod server;
mod overlay;

use std::sync::{Arc, RwLock};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, ShowWindow, TranslateMessage, SW_HIDE, SW_SHOWNA
};

use settings::load_settings;
use overlay::{create_overlay_window, AppState};

fn main() {
    // Enable DPI awareness to prevent Windows scaling artifacts
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetProcessDPIAware();
    }

    // 1. Load configuration
    let settings = Arc::new(RwLock::new(load_settings()));
    let hwnd_receiver = Arc::new(RwLock::new(None));

    // 2. Start the lightweight web server in background thread
    server::start_server(settings.clone(), hwnd_receiver.clone());

    // 3. Prepare window state on heap
    let state = Box::into_raw(Box::new(AppState {
        settings: settings.clone(),
    }));

    // 4. Create transparent click-through window
    let hwnd = create_overlay_window(state);

    // 5. Save HWND to the receiver to allow the web server to communicate
    *hwnd_receiver.write().unwrap() = Some(hwnd as isize);

    // 6. Set initial window visibility
    let enabled = settings.read().unwrap().enabled;
    unsafe {
        if enabled {
            ShowWindow(hwnd, SW_SHOWNA);
        } else {
            ShowWindow(hwnd, SW_HIDE);
        }
    }

    // 7. Standard Win32 message loop (handles hotkeys, tray, paints)
    unsafe {
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // 8. Clean up heap-allocated window state
    unsafe {
        let _ = Box::from_raw(state);
    }
}
