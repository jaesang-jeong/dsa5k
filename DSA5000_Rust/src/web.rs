// Embedded Web GUI Server & WebSocket Engine in Pure Rust (Zero Dependencies)

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::client::DSA5000Client;
use crate::logger::CsvLogger;
use crate::protocol::{Packet, ValveMode};
use crate::simulator::DSA5000Simulator;

const HTML: &str = include_str!("../web/index.html");
const CSS: &str = include_str!("../web/style.css");
const JS: &str = include_str!("../web/app.js");

pub struct WebServer {
    pub port: u16,
    ws_clients: Arc<Mutex<Vec<TcpStream>>>,
}

impl WebServer {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            ws_clients: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[allow(dead_code)]
    pub fn broadcast_ws(&self, msg: &str) {
        let frame = encode_ws_text_frame(msg);
        let mut clients = self.ws_clients.lock().unwrap();
        clients.retain_mut(|socket| socket.write_all(&frame).is_ok());
    }

    pub fn start(
        &mut self,
        client: Arc<Mutex<DSA5000Client>>,
        simulator: Arc<Mutex<Option<DSA5000Simulator>>>,
        logger: Arc<Mutex<CsvLogger>>,
    ) -> std::io::Result<u16> {
        let mut bind_port = self.port;
        let listener = loop {
            match TcpListener::bind(format!("0.0.0.0:{}", bind_port)) {
                Ok(l) => break l,
                Err(e) => {
                    if bind_port < self.port + 10 {
                        bind_port += 1;
                    } else {
                        return Err(e);
                    }
                }
            }
        };
        self.port = bind_port;
        println!("\x1b[1;36m[+] Embedded Web GUI Server listening on http://localhost:{}\x1b[0m", self.port);

        let ws_clients = Arc::clone(&self.ws_clients);

        thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(socket) = stream {
                    let ws_clients_clone = Arc::clone(&ws_clients);
                    let client_clone = Arc::clone(&client);
                    let sim_clone = Arc::clone(&simulator);
                    let logger_clone = Arc::clone(&logger);

                    thread::spawn(move || {
                        handle_http_connection(
                            socket,
                            ws_clients_clone,
                            client_clone,
                            sim_clone,
                            logger_clone,
                        );
                    });
                }
            }
        });

        Ok(self.port)
    }
}

fn handle_http_connection(
    mut socket: TcpStream,
    ws_clients: Arc<Mutex<Vec<TcpStream>>>,
    client: Arc<Mutex<DSA5000Client>>,
    simulator: Arc<Mutex<Option<DSA5000Simulator>>>,
    logger: Arc<Mutex<CsvLogger>>,
) {
    let mut buf = [0u8; 4096];
    let n = match socket.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let req = String::from_utf8_lossy(&buf[..n]);
    let mut lines = req.lines();
    let first_line = match lines.next() {
        Some(l) => l,
        None => return,
    };

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let method = parts[0];
    let path = parts[1];

    // Check for WebSocket Upgrade
    if req.contains("Upgrade: websocket") || req.contains("upgrade: websocket") {
        if let Some(key) = extract_header(&req, "Sec-WebSocket-Key") {
            let accept = generate_ws_accept(key.trim());
            let resp = format!(
                "HTTP/1.1 101 Switching Protocols\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Accept: {}\r\n\r\n",
                accept
            );
            if socket.write_all(resp.as_bytes()).is_ok() {
                // Send initial status frame
                let init_json = {
                    let c = client.lock().unwrap();
                    let info = c.system_info.lock().unwrap().clone();
                    let st = c.get_stats();
                    let is_sim = simulator.lock().unwrap().is_some();
                    format!(
                        r#"{{"type":"init","data":{{"isConnected":{},"isScanning":{},"systemInfo":{{"model":"{}","serialNumber":"{}","scanRateHz":{},"engineeringUnits":"{}","valveMode":"{}"}},"stats":{{"totalFrames":{},"droppedFrames":{},"currentFps":{}}},"isSimulatorRunning":{}}}}}"#,
                        c.is_connected(),
                        c.is_scanning(),
                        info.model,
                        info.serial_number,
                        info.scan_rate_hz,
                        info.engineering_units,
                        info.valve_mode,
                        st.total_frames,
                        st.dropped_frames,
                        st.current_fps,
                        is_sim
                    )
                };
                let init_frame = encode_ws_text_frame(&init_json);
                let _ = socket.write_all(&init_frame);

                ws_clients.lock().unwrap().push(socket);
            }
        }
        return;
    }

    // Static Assets
    if method == "GET" {
        match path {
            "/" | "/index.html" => {
                send_response(&mut socket, 200, "text/html; charset=utf-8", HTML.as_bytes());
            }
            "/style.css" => {
                send_response(&mut socket, 200, "text/css; charset=utf-8", CSS.as_bytes());
            }
            "/app.js" => {
                send_response(&mut socket, 200, "text/javascript; charset=utf-8", JS.as_bytes());
            }
            "/api/status" => {
                let c = client.lock().unwrap();
                let info = c.system_info.lock().unwrap().clone();
                let st = c.get_stats();
                let is_sim = simulator.lock().unwrap().is_some();
                let json = format!(
                    r#"{{"isConnected":{},"isScanning":{},"systemInfo":{{"model":"{}","serialNumber":"{}","scanRateHz":{},"engineeringUnits":"{}","valveMode":"{}"}},"stats":{{"totalFrames":{},"droppedFrames":{},"currentFps":{}}},"isSimulatorRunning":{}}}"#,
                    c.is_connected(),
                    c.is_scanning(),
                    info.model,
                    info.serial_number,
                    info.scan_rate_hz,
                    info.engineering_units,
                    info.valve_mode,
                    st.total_frames,
                    st.dropped_frames,
                    st.current_fps,
                    is_sim
                );
                send_response(&mut socket, 200, "application/json", json.as_bytes());
            }
            _ => {
                send_response(&mut socket, 404, "text/plain", b"404 Not Found");
            }
        }
        return;
    }

    // REST API POST Requests
    if method == "POST" {
        let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
        match path {
            "/api/connect" => {
                let mut c = client.lock().unwrap();
                let mut host = "127.0.0.1";
                let mut cmd_port = 2323u16;
                let mut data_port = 5503u16;

                // Simple JSON field extraction
                if let Some(h) = extract_json_str(body, "host") {
                    host = h;
                }
                if let Some(p) = extract_json_num(body, "cmdPort") {
                    cmd_port = p as u16;
                }
                if let Some(p) = extract_json_num(body, "dataPort") {
                    data_port = p as u16;
                }

                c.host = host.to_string();
                c.cmd_port = cmd_port;
                c.data_port = data_port;

                match c.connect() {
                    Ok(info) => {
                        let json = format!(
                            r#"{{"success":true,"systemInfo":{{"model":"{}","serialNumber":"{}","scanRateHz":{},"engineeringUnits":"{}","valveMode":"{}"}}}}"#,
                            info.model, info.serial_number, info.scan_rate_hz, info.engineering_units, info.valve_mode
                        );
                        send_response(&mut socket, 200, "application/json", json.as_bytes());
                    }
                    Err(e) => {
                        let json = format!(r#"{{"error":"{}"}}"#, e);
                        send_response(&mut socket, 500, "application/json", json.as_bytes());
                    }
                }
            }
            "/api/disconnect" => {
                let mut c = client.lock().unwrap();
                c.disconnect();
                send_response(&mut socket, 200, "application/json", b"{\"success\":true}");
            }
            "/api/start-scan" => {
                let mut c = client.lock().unwrap();
                let ws_clone = Arc::clone(&ws_clients);
                let logger_clone = Arc::clone(&logger);

                let res = c.start_scan(move |pkt| {
                    // Log to CSV
                    {
                        let mut l = logger_clone.lock().unwrap();
                        if l.is_recording {
                            let _ = l.write_frame(&pkt);
                        }
                    }

                    // Broadcast to WebSocket at ~60Hz
                    if pkt.header.frame_number % 2 == 0 {
                        let json = format_packet_json(&pkt);
                        let frame = encode_ws_text_frame(&json);
                        let mut clients = ws_clone.lock().unwrap();
                        clients.retain_mut(|s| s.write_all(&frame).is_ok());
                    }
                });

                match res {
                    Ok(_) => send_response(&mut socket, 200, "application/json", b"{\"success\":true}"),
                    Err(e) => {
                        let json = format!(r#"{{"error":"{}"}}"#, e);
                        send_response(&mut socket, 500, "application/json", json.as_bytes());
                    }
                }
            }
            "/api/stop-scan" => {
                let mut c = client.lock().unwrap();
                c.stop_scan();
                send_response(&mut socket, 200, "application/json", b"{\"success\":true}");
            }
            "/api/valve" => {
                let mode_str = extract_json_str(body, "mode").unwrap_or("PX");
                let mut c = client.lock().unwrap();
                if let Some(mode) = ValveMode::from_str(mode_str) {
                    let _ = c.set_valve(mode);
                    let json = format!(r#"{{"success":true,"mode":"{}"}}"#, mode.as_str());
                    send_response(&mut socket, 200, "application/json", json.as_bytes());
                } else {
                    send_response(&mut socket, 400, "application/json", b"{\"error\":\"Invalid mode\"}");
                }
            }
            "/api/calz" => {
                let c_clone = Arc::clone(&client);
                let ws_clone = Arc::clone(&ws_clients);
                thread::spawn(move || {
                    let mut c = c_clone.lock().unwrap();
                    let _ = c.zero_calibration(|status, pct| {
                        let json = format!(r#"{{"type":"calzProgress","data":{{"status":"{}","percent":{}}}}}"#, status, pct);
                        let frame = encode_ws_text_frame(&json);
                        let mut clients = ws_clone.lock().unwrap();
                        clients.retain_mut(|s| s.write_all(&frame).is_ok());
                    });
                });
                send_response(&mut socket, 200, "application/json", b"{\"success\":true}");
            }
            "/api/settings" => {
                let mut c = client.lock().unwrap();
                if let Some(rate) = extract_json_num(body, "rateHz") {
                    let _ = c.set_rate(rate as u32);
                }
                if let Some(units) = extract_json_str(body, "units") {
                    let _ = c.set_units(units);
                }
                send_response(&mut socket, 200, "application/json", b"{\"success\":true}");
            }
            "/api/logging/start" => {
                let mut l = logger.lock().unwrap();
                let c = client.lock().unwrap();
                let info = c.system_info.lock().unwrap().clone();
                let name = extract_json_str(body, "fileName").unwrap_or("DSA5000_Scan");
                match l.start(&info, name) {
                    Ok(p) => {
                        let json = format!(r#"{{"success":true,"filePath":"{:?}"}}"#, p);
                        send_response(&mut socket, 200, "application/json", json.as_bytes());
                    }
                    Err(e) => {
                        let json = format!(r#"{{"error":"{}"}}"#, e);
                        send_response(&mut socket, 500, "application/json", json.as_bytes());
                    }
                }
            }
            "/api/logging/stop" => {
                let mut l = logger.lock().unwrap();
                match l.stop() {
                    Ok(p) => {
                        let json = format!(r#"{{"success":true,"filePath":"{:?}"}}"#, p);
                        send_response(&mut socket, 200, "application/json", json.as_bytes());
                    }
                    Err(e) => {
                        let json = format!(r#"{{"error":"{}"}}"#, e);
                        send_response(&mut socket, 500, "application/json", json.as_bytes());
                    }
                }
            }
            "/api/terminal" => {
                let cmd = extract_json_str(body, "command").unwrap_or("");
                let mut c = client.lock().unwrap();
                match c.send_command(cmd) {
                    Ok(resp) => {
                        let clean = resp.replace('"', "\\\"").replace('\r', "").replace('\n', "\\n");
                        let json = format!(r#"{{"success":true,"response":"{}"}}"#, clean);
                        send_response(&mut socket, 200, "application/json", json.as_bytes());
                    }
                    Err(e) => {
                        let json = format!(r#"{{"error":"{}"}}"#, e);
                        send_response(&mut socket, 500, "application/json", json.as_bytes());
                    }
                }
            }
            "/api/simulator/start" => {
                let mut sim_guard = simulator.lock().unwrap();
                if sim_guard.is_none() {
                    let mut sim = DSA5000Simulator::new(2323, 5503);
                    let _ = sim.start();
                    *sim_guard = Some(sim);
                }
                send_response(&mut socket, 200, "application/json", b"{\"success\":true}");
            }
            "/api/simulator/stop" => {
                let mut sim_guard = simulator.lock().unwrap();
                if let Some(mut s) = sim_guard.take() {
                    s.stop();
                }
                send_response(&mut socket, 200, "application/json", b"{\"success\":true}");
            }
            _ => {
                send_response(&mut socket, 404, "application/json", b"{\"error\":\"Not found\"}");
            }
        }
    }
}

fn send_response(socket: &mut TcpStream, code: u16, content_type: &str, body: &[u8]) {
    let headers = format!(
        "HTTP/1.1 {} OK\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Connection: close\r\n\r\n",
        code,
        content_type,
        body.len()
    );
    let _ = socket.write_all(headers.as_bytes());
    let _ = socket.write_all(body);
    let _ = socket.flush();
}

fn format_packet_json(pkt: &Packet) -> String {
    let mut p_arr = String::new();
    if let Some(m0) = pkt.modules.first() {
        for (i, p) in m0.pressures.iter().enumerate() {
            if i > 0 {
                p_arr.push(',');
            }
            p_arr.push_str(&format!("{:.4}", p));
        }
    }
    format!(
        r#"{{"type":"frame","data":{{"frameNumber":{},"ptpSec":{},"ptpNsec":{},"timestampMs":{},"modules":[{{"pressures":[{}]}}]}}}}"#,
        pkt.header.frame_number,
        pkt.header.ptp_sec,
        pkt.header.ptp_nsec,
        pkt.timestamp_ms,
        p_arr
    )
}

fn extract_header<'a>(req: &'a str, name: &str) -> Option<&'a str> {
    for line in req.lines() {
        if line.to_lowercase().starts_with(&name.to_lowercase()) {
            return line.split(':').nth(1).map(|v| v.trim());
        }
    }
    None
}

fn extract_json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern)?;
    let rest = &json[pos + pattern.len()..];
    let col = rest.find(':')?;
    let val_start = rest[col + 1..].find('"')? + col + 2;
    let val_end = rest[val_start..].find('"')? + val_start;
    Some(&rest[val_start..val_end])
}

fn extract_json_num(json: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern)?;
    let rest = &json[pos + pattern.len()..];
    let col = rest.find(':')?;
    let slice = &rest[col + 1..].trim_start();
    let end = slice
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(slice.len());
    slice[..end].parse().ok()
}

// RFC 6455 WebSocket Handshake Helpers
fn generate_ws_accept(key: &str) -> String {
    const MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let combined = format!("{}{}", key, MAGIC);
    let hash = sha1(combined.as_bytes());
    base64_encode(&hash)
}

fn encode_ws_text_frame(payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    let len = bytes.len();
    let mut frame = Vec::with_capacity(len + 10);

    frame.push(0x81); // Text frame FIN

    if len <= 125 {
        frame.push(len as u8);
    } else if len <= 65535 {
        frame.push(126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }

    frame.extend_from_slice(bytes);
    frame
}

// Pure Rust SHA-1 (RFC 3174)
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };

            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut res = [0u8; 20];
    res[0..4].copy_from_slice(&h0.to_be_bytes());
    res[4..8].copy_from_slice(&h1.to_be_bytes());
    res[8..12].copy_from_slice(&h2.to_be_bytes());
    res[12..16].copy_from_slice(&h3.to_be_bytes());
    res[16..20].copy_from_slice(&h4.to_be_bytes());
    res
}

// Pure Rust Base64
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);

        if chunk.len() > 1 {
            out.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }

        if chunk.len() > 2 {
            out.push(CHARS[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
