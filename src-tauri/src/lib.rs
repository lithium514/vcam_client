use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use ssh2::KeyboardInteractivePrompt;
use tauri::{AppHandle, Emitter, Manager, State};

struct PasswordPrompt(String);

impl KeyboardInteractivePrompt for PasswordPrompt {
    fn prompt<'a>(&mut self, _name: &str, _instruction: &str, prompts: &[ssh2::Prompt<'a>]) -> Vec<String> {
        prompts.iter().map(|_| self.0.clone()).collect()
    }
}

fn find_binary(app: &AppHandle, name: &str) -> String {
    if let Ok(resource_dir) = app.path().resource_dir() {
        for candidate in [name, &format!("{name}.exe")] {
            let path = resource_dir.join(candidate);
            if path.exists() {
                return path.to_string_lossy().to_string();
            }
        }
        // fallback: search subdirectories recursively
        if let Ok(entries) = resource_dir.read_dir() {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    for candidate in [name, &format!("{name}.exe")] {
                        let path = entry.path().join(candidate);
                        if path.exists() {
                            return path.to_string_lossy().to_string();
                        }
                    }
                }
            }
        }
    }
    name.to_string()
}

// ── SSH tunnel via ssh2 (no terminal window, password auth) ──

struct TunnelState {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    local_port: u16,
}

struct SshTunnel(Mutex<Option<TunnelState>>);

#[tauri::command]
fn start_ssh_tunnel(
    app: AppHandle,
    host: String,
    port: u16,
    user: String,
    password: String,
    local_port: u16,
    key_path: Option<String>,
    key_passphrase: Option<String>,
    state: State<SshTunnel>,
) -> Result<String, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Err("SSH tunnel is already running".into());
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let local_port_owned = local_port;
    let ready = Arc::new(AtomicBool::new(false));
    let ready_clone = ready.clone();

    let handle = thread::spawn(move || {
        if let Err(e) = run_tunnel(&host, port, &user, &password, key_path.as_deref(), key_passphrase.as_deref(), local_port_owned, &stop_clone, &ready_clone) {
            eprintln!("SSH tunnel error: {e}");
        }
    });

    // wait for the tunnel to signal readiness (up to 15s)
    for _ in 0..75 {
        if ready.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if !ready.load(Ordering::SeqCst) {
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
        return Err("SSH tunnel failed to start within 15s".into());
    }

    let adb_path = find_binary(&app, "adb");
    let adb_output = Command::new(adb_path)
        .arg("connect")
        .arg(format!("127.0.0.1:{local_port}"))
        .output()
        .map_err(|e| format!("Failed to run adb connect: {e}"))?;

    let adb_msg = String::from_utf8_lossy(&adb_output.stdout).trim().to_string();
    let adb_err = String::from_utf8_lossy(&adb_output.stderr).trim().to_string();
    let combined = if adb_err.is_empty() { adb_msg } else { format!("{adb_msg} {adb_err}") };

    guard.replace(TunnelState { stop, handle: Some(handle), local_port });
    Ok(format!("SSH tunnel established on localhost:{local_port}\n{combined}"))
}

fn run_tunnel(host: &str, port: u16, user: &str, password: &str, key_path: Option<&str>, key_passphrase: Option<&str>, local_port: u16, stop: &AtomicBool, ready: &AtomicBool) -> Result<(), String> {
    let tcp = TcpStream::connect(format!("{host}:{port}"))
        .map_err(|e| format!("TCP connect failed: {e}"))?;

    let mut sess = ssh2::Session::new().map_err(|e| format!("ssh2 init: {e}"))?;
    sess.set_tcp_stream(tcp);
    sess.handshake().map_err(|e| format!("SSH handshake: {e}"))?;

    // try agent first
    let mut authed = false;
    if sess.userauth_agent(user).is_ok() && sess.authenticated() {
        authed = true;
    }

    // try public key if key_path provided
    if !authed {
        if let Some(kp) = key_path {
            if !kp.is_empty() {
                let pass: Option<&str> = key_passphrase.and_then(|p| if p.is_empty() { None } else { Some(p) });
                let pubkey_str = format!("{kp}.pub");
                let pubkey_path = if kp.ends_with(".ppk") { None } else { Some(std::path::Path::new(&pubkey_str)) };
                if sess.userauth_pubkey_file(user, pubkey_path, std::path::Path::new(kp), pass).is_ok() && sess.authenticated() {
                    authed = true;
                }
            }
        }
    }

    // fall back to password
    if !authed {
        if sess.userauth_password(user, password).is_ok() && sess.authenticated() {
            authed = true;
        }
        // some servers require keyboard-interactive for password
        if !authed {
            let pw = password.to_string();
            if sess.userauth_keyboard_interactive(user, &mut PasswordPrompt(pw)).is_ok() && sess.authenticated() {
                authed = true;
            }
        }
    }

    if !authed {
        return Err("SSH authentication failed (tried agent, pubkey, password)".into());
    }

    let listener = TcpListener::bind(format!("127.0.0.1:{local_port}"))
        .map_err(|e| format!("Cannot bind local port {local_port}: {e}"))?;
    listener.set_nonblocking(true).ok();

    // signal that the tunnel is ready
    ready.store(true, Ordering::SeqCst);

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((conn, _)) => {
                let channel = match sess.channel_direct_tcpip("localhost", 5555, None) {
                    Ok(ch) => ch,
                    Err(_) => continue,
                };
                thread::spawn(|| forward_connection(conn, channel));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(_) => break,
        }
    }

    Ok(())
}

fn forward_connection(mut tcp: TcpStream, channel: ssh2::Channel) {
    let channel = Arc::new(Mutex::new(channel));
    let chan2 = channel.clone();
    let mut tcp_clone = match tcp.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    };

    let t = thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match tcp_clone.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            let mut ch = chan2.lock().unwrap();
            if ch.write(&buf[..n]).is_err() {
                break;
            }
            ch.flush().ok();
        }
    });

    let mut buf = [0u8; 8192];
    loop {
        let n = {
            let mut ch = channel.lock().unwrap();
            match ch.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            }
        };
        if tcp.write_all(&buf[..n]).is_err() {
            break;
        }
    }

    t.join().ok();
}

#[tauri::command]
fn stop_ssh_tunnel(app: AppHandle, state: State<SshTunnel>) -> Result<String, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;

    match guard.take() {
        Some(TunnelState { stop, handle, local_port }) => {
            stop.store(true, Ordering::SeqCst);
            if let Some(h) = handle {
                h.join().ok();
            }

            let adb_path = find_binary(&app, "adb");
            let _ = Command::new(adb_path)
                .arg("disconnect")
                .arg(format!("127.0.0.1:{local_port}"))
                .output();

            Ok("SSH tunnel stopped, ADB disconnected".into())
        }
        None => Err("No SSH tunnel running".into()),
    }
}

#[tauri::command]
fn get_tunnel_status(state: State<SshTunnel>) -> Result<bool, String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    Ok(guard.is_some())
}

// ── Image upload (bypasses CORS) ──

#[tauri::command]
fn upload_image(url: String, data: Vec<u8>) -> Result<String, String> {
    let boundary = format!("----WebKitFormBoundary{}", rand_boundary());

    let mut body = Vec::new();
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"image\"; filename=\"photo.png\"\r\n");
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(&data);
    body.extend_from_slice(b"\r\n--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");

    let resp = ureq::post(&url)
        .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .send(body)
        .map_err(|e| format!("Upload failed: {e}"))?;

    let status = resp.status();
    Ok(format!("HTTP {status}"))
}

fn rand_boundary() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("boundary_{nanos}")
}

// ── scrcpy launcher ──

#[cfg(target_os = "windows")]
fn hide_window() -> u32 {
    0x08000000 // CREATE_NO_WINDOW
}
#[cfg(not(target_os = "windows"))]
fn hide_window() -> u32 {
    0
}

#[tauri::command]
fn launch_scrcpy(app: AppHandle, audio_codec: Option<String>, audio_encoder: Option<String>) -> Result<String, String> {
    let scrcpy_path = find_binary(&app, "scrcpy");
    let mut cmd = Command::new(scrcpy_path);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(hide_window());
    }

    if let Some(codec) = &audio_codec {
        if !codec.is_empty() {
            cmd.arg("--audio-codec").arg(codec);
        }
    }
    if let Some(encoder) = &audio_encoder {
        if !encoder.is_empty() {
            cmd.arg("--audio-encoder").arg(encoder);
        }
    }

    let child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch scrcpy: {e}"))?;

    std::mem::drop(child);
    Ok("scrcpy launched".into())
}

// ── Screencap streaming ──

struct ScreencapRunning(Arc<AtomicBool>);

#[tauri::command]
fn start_screencap(
    app: AppHandle,
    state: State<ScreencapRunning>,
) -> Result<String, String> {
    if state.0.load(Ordering::SeqCst) {
        return Err("Screencap is already running".into());
    }

    state.0.store(true, Ordering::SeqCst);
    let running = state.0.clone();

    thread::spawn(move || {
        let adb_path = find_binary(&app, "adb");
        while running.load(Ordering::SeqCst) {
            let output = Command::new(&adb_path)
                .args(["exec-out", "screencap", "-p"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output();

            match output {
                Ok(out) if out.status.success() && !out.stdout.is_empty() => {
                    let b64 = base64_encode(&out.stdout);
                    let _ = app.emit("screencap-frame", &b64);
                }
                _ => {
                    running.store(false, Ordering::SeqCst);
                    let _ = app.emit("screencap-error", "Screencap failed");
                    break;
                }
            }
        }
    });

    Ok("Screencap started".into())
}

#[tauri::command]
fn stop_screencap(state: State<ScreencapRunning>) -> Result<String, String> {
    state.0.store(false, Ordering::SeqCst);
    Ok("Screencap stopped".into())
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(SshTunnel(Mutex::new(None)))
        .manage(ScreencapRunning(Arc::new(AtomicBool::new(false))))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            start_ssh_tunnel,
            stop_ssh_tunnel,
            get_tunnel_status,
            start_screencap,
            stop_screencap,
            launch_scrcpy,
            upload_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
