use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter, State};

struct TunnelState {
    child: Child,
    local_port: u16,
}

struct SshTunnel(Mutex<Option<TunnelState>>);

#[tauri::command]
fn start_ssh_tunnel(
    host: String,
    port: u16,
    user: String,
    local_port: u16,
    state: State<SshTunnel>,
) -> Result<String, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;

    if guard.is_some() {
        return Err("SSH tunnel is already running".into());
    }

    let child = Command::new("ssh")
        .arg("-L")
        .arg(format!("{local_port}:localhost:5555"))
        .arg("-p")
        .arg(port.to_string())
        .arg("-N")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg(format!("{}@{}", user, host))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn ssh: {e}"))?;

    guard.replace(TunnelState { child, local_port });

    std::thread::sleep(std::time::Duration::from_secs(1));

    let adb_output = Command::new("adb")
        .arg("connect")
        .arg(format!("localhost:{local_port}"))
        .output()
        .map_err(|e| format!("Failed to run adb connect: {e}"))?;

    let adb_msg = String::from_utf8_lossy(&adb_output.stdout).trim().to_string();
    let adb_err = String::from_utf8_lossy(&adb_output.stderr).trim().to_string();
    let combined = if adb_err.is_empty() {
        adb_msg
    } else {
        format!("{adb_msg} {adb_err}")
    };

    Ok(format!(
        "SSH tunnel established on localhost:{local_port}\n{combined}"
    ))
}

#[tauri::command]
fn stop_ssh_tunnel(state: State<SshTunnel>) -> Result<String, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;

    match guard.take() {
        Some(TunnelState { mut child, local_port }) => {
            child.kill().map_err(|e| format!("Failed to kill ssh: {e}"))?;
            child.wait().ok();

            let _ = Command::new("adb")
                .arg("disconnect")
                .arg(format!("localhost:{local_port}"))
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
    // header
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

#[tauri::command]
fn launch_scrcpy(audio_codec: Option<String>, audio_encoder: Option<String>) -> Result<String, String> {
    let mut cmd = Command::new("scrcpy");

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
        while running.load(Ordering::SeqCst) {
            let output = Command::new("adb")
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
                    // device disconnected or error, stop loop
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
