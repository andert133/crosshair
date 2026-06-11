use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::thread;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::settings::{save_settings, set_autostart, Settings};

// Custom Win32 window messages
pub const WM_UPDATE_SETTINGS: u32 = 0x0400 + 1;   // WM_USER + 1
pub const WM_TOGGLE_VISIBILITY: u32 = 0x0400 + 2; // WM_USER + 2
pub const WM_EXIT_APPLICATION: u32 = 0x0400 + 3;  // WM_USER + 3

const HTML_CONTENT: &str = include_str!("index.html");

pub fn start_server(settings: Arc<RwLock<Settings>>, hwnd_receiver: Arc<RwLock<Option<isize>>>) {
    thread::spawn(move || {
        let listener = loop {
            match TcpListener::bind("127.0.0.1:25432") {
                Ok(l) => break l,
                Err(_) => {
                    thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        };

        for stream in listener.incoming() {
            if let Ok(stream) = stream {
                let settings_clone = settings.clone();
                let hwnd_clone = hwnd_receiver.clone();
                thread::spawn(move || {
                    handle_connection(stream, settings_clone, hwnd_clone);
                });
            }
        }
    });
}

fn handle_connection(
    mut stream: TcpStream,
    settings: Arc<RwLock<Settings>>,
    hwnd_receiver: Arc<RwLock<Option<isize>>>,
) {
    let mut buffer = [0; 8192];
    let mut bytes_read = 0;

    // Read headers
    while bytes_read < buffer.len() {
        match stream.read(&mut buffer[bytes_read..]) {
            Ok(0) => break,
            Ok(n) => {
                bytes_read += n;
                if let Some(pos) = find_subsequence(&buffer[..bytes_read], b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&buffer[..pos]);
                    
                    // Parse Content-Length
                    let mut content_length = 0;
                    for line in headers.lines() {
                        if line.to_lowercase().starts_with("content-length:") {
                            if let Some(len_str) = line.split(':').nth(1) {
                                content_length = len_str.trim().parse::<usize>().unwrap_or(0);
                            }
                        }
                    }

                    let body_start = pos + 4;
                    let mut body = buffer[body_start..bytes_read].to_vec();

                    // Read rest of body if necessary
                    while body.len() < content_length {
                        let mut temp = vec![0; content_length - body.len()];
                        match stream.read(&mut temp) {
                            Ok(0) => break,
                            Ok(n) => {
                                body.extend_from_slice(&temp[..n]);
                            }
                            Err(_) => break,
                        }
                    }

                    // Parse Request Line
                    let req_line = headers.lines().next().unwrap_or("");
                    let parts: Vec<&str> = req_line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let method = parts[0];
                        let path = parts[1];

                        route_request(&mut stream, method, path, &body, settings, hwnd_receiver);
                    }
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn route_request(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    body: &[u8],
    settings: Arc<RwLock<Settings>>,
    hwnd_receiver: Arc<RwLock<Option<isize>>>,
) {
    match (method, path) {
        ("GET", "/") => {
            send_response(stream, 200, "text/html; charset=utf-8", HTML_CONTENT.as_bytes());
        }
        ("GET", "/api/settings") => {
            let current_settings = settings.read().unwrap();
            if let Ok(json) = serde_json::to_string(&*current_settings) {
                send_response(stream, 200, "application/json", json.as_bytes());
            } else {
                send_response(stream, 500, "text/plain", b"Internal Server Error");
            }
        }
        ("POST", "/api/settings") => {
            if let Ok(new_settings) = serde_json::from_slice::<Settings>(body) {
                let mut autostart_changed = false;
                let mut autostart_val = false;

                {
                    let mut current_settings = settings.write().unwrap();
                    
                    if current_settings.autostart != new_settings.autostart {
                        autostart_changed = true;
                        autostart_val = new_settings.autostart;
                    }

                    *current_settings = new_settings;
                    
                    // Save to disk
                    let _ = save_settings(&*current_settings);
                }

                // If autostart was toggled, update registry
                if autostart_changed {
                    let _ = set_autostart(autostart_val);
                }

                // Notify UI thread
                if let Some(hwnd) = *hwnd_receiver.read().unwrap() {
                    unsafe {
                        PostMessageW(hwnd as _, WM_UPDATE_SETTINGS, 0, 0);
                    }
                }

                send_response(stream, 200, "application/json", b"{\"status\":\"ok\"}");
            } else {
                send_response(stream, 400, "text/plain", b"Bad Request");
            }
        }
        ("POST", "/api/toggle") => {
            let new_state = {
                let mut current_settings = settings.write().unwrap();
                current_settings.enabled = !current_settings.enabled;
                let _ = save_settings(&*current_settings);
                current_settings.enabled
            };

            // Notify UI thread
            if let Some(hwnd) = *hwnd_receiver.read().unwrap() {
                unsafe {
                    PostMessageW(hwnd as _, WM_TOGGLE_VISIBILITY, 0, 0);
                }
            }

            let response_json = format!("{{\"enabled\":{}}}", new_state);
            send_response(stream, 200, "application/json", response_json.as_bytes());
        }
        ("POST", "/api/exit") => {
            // Notify UI thread to exit
            if let Some(hwnd) = *hwnd_receiver.read().unwrap() {
                unsafe {
                    PostMessageW(hwnd as _, WM_EXIT_APPLICATION, 0, 0);
                }
            }
            send_response(stream, 200, "application/json", b"{\"status\":\"exiting\"}");
        }
        _ => {
            send_response(stream, 404, "text/plain", b"Not Found");
        }
    }
}

fn send_response(stream: &mut TcpStream, status_code: u16, content_type: &str, body: &[u8]) {
    let status_text = match status_code {
        200 => "OK",
        400 => "Bad Request",
        444 => "No Response",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };

    let response_headers = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Cache-Control: no-cache, no-store, must-revalidate\r\n\
         Pragma: no-cache\r\n\
         Expires: 0\r\n\
         \r\n",
        status_code,
        status_text,
        content_type,
        body.len()
    );

    let _ = stream.write_all(response_headers.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
