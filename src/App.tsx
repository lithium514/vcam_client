import { useRef, useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Tab = "camera" | "scrcpy";

function CameraTab({ uploadUrl }: { uploadUrl: string }) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [cameras, setCameras] = useState<MediaDeviceInfo[]>([]);
  const [selectedDeviceId, setSelectedDeviceId] = useState(() => localStorage.getItem("camera_deviceId") || "");
  const [stream, setStream] = useState<MediaStream | null>(null);
  const [capturedBlob, setCapturedBlob] = useState<Blob | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [uploading, setUploading] = useState(false);
  const [message, setMessage] = useState("");

  useEffect(() => {
    navigator.mediaDevices.enumerateDevices().then((devices) => {
      const videoDevices = devices.filter((d) => d.kind === "videoinput");
      setCameras(videoDevices);
      if (!selectedDeviceId && videoDevices.length > 0) {
        const id = videoDevices[0].deviceId;
        setSelectedDeviceId(id);
        localStorage.setItem("camera_deviceId", id);
      }
    });
  }, []);

  useEffect(() => {
    if (selectedDeviceId) startCamera(selectedDeviceId);
    return () => stopCamera();
  }, [selectedDeviceId]);

  async function startCamera(deviceId: string) {
    stopCamera();
    try {
      const s = await navigator.mediaDevices.getUserMedia({
        video: deviceId ? { deviceId: { exact: deviceId } } : true,
        audio: false,
      });
      setStream(s);
      if (videoRef.current) videoRef.current.srcObject = s;
    } catch {
      setMessage("Camera access denied");
    }
  }

  function stopCamera() {
    if (stream) {
      stream.getTracks().forEach((t) => t.stop());
      setStream(null);
    }
  }

  function onCameraChange(e: React.ChangeEvent<HTMLSelectElement>) {
    const id = e.target.value;
    setSelectedDeviceId(id);
    localStorage.setItem("camera_deviceId", id);
  }

  const capture = useCallback(() => {
    const video = videoRef.current;
    const canvas = canvasRef.current;
    if (!video || !canvas) return;

    canvas.width = video.videoWidth;
    canvas.height = video.videoHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.drawImage(video, 0, 0);
    canvas.toBlob((blob) => {
      if (blob) {
        setCapturedBlob(blob);
        setPreviewUrl(URL.createObjectURL(blob));
      }
    }, "image/png");
  }, []);

  async function upload() {
    if (!capturedBlob) return;
    setUploading(true);
    setMessage("");

    try {
      const buffer = await capturedBlob.arrayBuffer();
      const data = new Uint8Array(buffer);
      const result = await invoke<string>("upload_image", {
        url: uploadUrl,
        data: Array.from(data),
      });
      setMessage(`Upload successful (${result})`);
      setCapturedBlob(null);
      setPreviewUrl(null);
    } catch (err) {
      setMessage(`Upload error: ${err}`);
    } finally {
      setUploading(false);
    }
  }

  function retake() {
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    setCapturedBlob(null);
    setPreviewUrl(null);
    setMessage("");
  }

  useEffect(() => {
    return () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    };
  }, [previewUrl]);

  return (
    <div className="tab-content">
      <div className="viewfinder">
        <video ref={videoRef} autoPlay playsInline />
        <canvas ref={canvasRef} style={{ display: "none" }} />
        {previewUrl && (
          <img src={previewUrl} className="preview" alt="captured" />
        )}
      </div>

      <div className="upload-url-row">
        <span className="url-label">Server</span>
        <span className="url-value">{uploadUrl}</span>
      </div>

      <div className="controls">
        {!capturedBlob ? (
          <>
            <select className="camera-select" value={selectedDeviceId} onChange={onCameraChange}>
              {cameras.map((c) => (
                <option key={c.deviceId} value={c.deviceId}>{c.label || `Camera ${c.deviceId.slice(0, 8)}`}</option>
              ))}
            </select>
            <button className="shutter" onClick={capture}>
              📷 Capture
            </button>
          </>
        ) : (
          <>
            <button className="btn" onClick={retake}>Retake</button>
            <button className="btn upload-btn" onClick={upload} disabled={uploading}>
              {uploading ? "Uploading..." : "Upload"}
            </button>
          </>
        )}
      </div>

      {message && <p className="message">{message}</p>}
    </div>
  );
}

function ScrcpyTab() {
  const [host, setHost] = useState(() => localStorage.getItem("scrcpy_host") || "");
  const [port, setPort] = useState(() => localStorage.getItem("scrcpy_port") || "22");
  const [user, setUser] = useState(() => localStorage.getItem("scrcpy_user") || "");
  const [localPort, setLocalPort] = useState(() => localStorage.getItem("scrcpy_localPort") || "5555");
  const [audioCodec, setAudioCodec] = useState(() => localStorage.getItem("scrcpy_audioCodec") || "aac");
  const [audioEncoder, setAudioEncoder] = useState(() => localStorage.getItem("scrcpy_audioEncoder") || "");
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [streaming, setStreaming] = useState(false);
  const [frame, setFrame] = useState("");
  const [message, setMessage] = useState("");

  useEffect(() => {
    invoke<boolean>("get_tunnel_status").then(setConnected).catch(() => {});
  }, []);

  useEffect(() => {
    const unlisten = listen<string>("screencap-frame", (e) => {
      setFrame(`data:image/png;base64,${e.payload}`);
    });
    const unlistenErr = listen<string>("screencap-error", (e) => {
      setStreaming(false);
      setMessage(e.payload);
    });
    return () => {
      unlisten.then((f) => f());
      unlistenErr.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (!connected) {
      setStreaming(false);
      setFrame("");
      invoke("stop_screencap").catch(() => {});
    }
  }, [connected]);

  async function connect() {
    if (!host || !user) {
      setMessage("Host and user are required");
      return;
    }
    setConnecting(true);
    setMessage("");
    try {
      const res = await invoke<string>("start_ssh_tunnel", {
        host,
        port: parseInt(port) || 22,
        user,
        localPort: parseInt(localPort) || 5555,
      });
      setConnected(true);
      setMessage(res);

      // auto-launch scrcpy after tunnel established
      try {
        await invoke("launch_scrcpy", { audioCodec, audioEncoder });
        setMessage(res + "\nscrcpy launched");
      } catch (e) {
        setMessage(res + `\nscrcpy launch failed: ${e}`);
      }
    } catch (err) {
      setMessage(`Failed: ${err}`);
    } finally {
      setConnecting(false);
    }
  }

  async function launchScrcpy() {
    try {
      await invoke("launch_scrcpy", { audioCodec, audioEncoder });
      setMessage("scrcpy launched");
    } catch (err) {
      setMessage(`scrcpy launch failed: ${err}`);
    }
  }

  function setAndPersistCodec(v: string) {
    setAudioCodec(v);
    localStorage.setItem("scrcpy_audioCodec", v);
  }

  function setAndPersistEncoder(v: string) {
    setAudioEncoder(v);
    localStorage.setItem("scrcpy_audioEncoder", v);
  }

  async function disconnect() {
    setStreaming(false);
    setFrame("");
    await invoke("stop_screencap").catch(() => {});
    setMessage("");
    try {
      const res = await invoke<string>("stop_ssh_tunnel");
      setConnected(false);
      setMessage(res);
    } catch (err) {
      setMessage(`Error: ${err}`);
    }
  }

  async function startStream() {
    setMessage("");
    try {
      await invoke("start_screencap");
      setStreaming(true);
    } catch (err) {
      setMessage(`Failed to start stream: ${err}`);
    }
  }

  async function stopStream() {
    setFrame("");
    setStreaming(false);
    await invoke("stop_screencap").catch(() => {});
  }

  return (
    <div className="tab-content">
      <div className="ssh-form">
        <input value={host} onChange={(e) => { setHost(e.target.value); localStorage.setItem("scrcpy_host", e.target.value); }} placeholder="SSH Host" className="input" disabled={connected} />
        <input value={port} onChange={(e) => { setPort(e.target.value); localStorage.setItem("scrcpy_port", e.target.value); }} placeholder="Port" className="input input-sm" disabled={connected} />
        <input value={user} onChange={(e) => { setUser(e.target.value); localStorage.setItem("scrcpy_user", e.target.value); }} placeholder="SSH User" className="input" disabled={connected} />
        <input value={localPort} onChange={(e) => { setLocalPort(e.target.value); localStorage.setItem("scrcpy_localPort", e.target.value); }} placeholder="Local Port" className="input input-sm" disabled={connected} />
      </div>

      <div className="controls">
        {!connected ? (
          <button className="btn connect-btn" onClick={connect} disabled={connecting}>
            {connecting ? "Connecting..." : "Connect SSH"}
          </button>
        ) : (
          <button className="btn disconnect-btn" onClick={disconnect}>
            Disconnect
          </button>
        )}
      </div>

      <details className="scrcpy-options">
        <summary className="options-summary">Audio options</summary>
        <div className="options-grid">
          <label className="option-field">
            <span>Codec</span>
            <input value={audioCodec} onChange={(e) => setAndPersistCodec(e.target.value)} className="input" placeholder="aac" />
          </label>
          <label className="option-field">
            <span>Encoder</span>
            <input value={audioEncoder} onChange={(e) => setAndPersistEncoder(e.target.value)} className="input" placeholder="OMX.google.aac.encoder" />
          </label>
        </div>
      </details>

      <div className="tunnel-status">
        <span className={`status-dot ${connected ? "green" : "red"}`} />
        <span>{connected ? "Tunnel active" : "Not connected"}</span>
      </div>

      {connected && (
        <>
          <div className="viewfinder scrcpy-view">
            {frame ? (
              <img src={frame} className="scrcpy-feed" alt="device screen" />
            ) : (
              <div className="placeholder">
                {streaming ? "Waiting for frame..." : "Click Start Preview to begin"}
              </div>
            )}
          </div>

          <div className="btn-row">
            <button
              className={`btn ${streaming ? "disconnect-btn" : "connect-btn"}`}
              onClick={streaming ? stopStream : startStream}
            >
              {streaming ? "Stop Preview" : "Start Preview"}
            </button>
            <button className="btn scrcpy-btn" onClick={launchScrcpy}>
              Launch scrcpy
            </button>
          </div>
        </>
      )}

      {message && <p className="message">{message}</p>}
    </div>
  );
}

function App() {
  const [tab, setTab] = useState<Tab>("camera");
  const [uploadUrl, setUploadUrl] = useState(
    () => localStorage.getItem("uploadUrl") || "http://127.0.0.1:5000/upload"
  );
  const [showSettings, setShowSettings] = useState(false);

  function handleUrlChange(url: string) {
    setUploadUrl(url);
    localStorage.setItem("uploadUrl", url);
  }

  return (
    <div className="app">
      <div className="header">
        <h1>vCam</h1>
        <button
          className="settings-btn"
          onClick={() => setShowSettings(!showSettings)}
        >
          ⚙
        </button>
      </div>

      {showSettings && (
        <div className="settings-panel">
          <label className="settings-label">Upload URL</label>
          <input
            value={uploadUrl}
            onChange={(e) => handleUrlChange(e.target.value)}
            className="input"
            placeholder="http://127.0.0.1:5000/upload"
          />
        </div>
      )}

      <div className="tabs">
        <button
          className={`tab ${tab === "camera" ? "active" : ""}`}
          onClick={() => setTab("camera")}
        >
          Camera
        </button>
        <button
          className={`tab ${tab === "scrcpy" ? "active" : ""}`}
          onClick={() => setTab("scrcpy")}
        >
          scrcpy
        </button>
      </div>

      {tab === "camera" ? <CameraTab uploadUrl={uploadUrl} /> : <ScrcpyTab />}
    </div>
  );
}

export default App;
