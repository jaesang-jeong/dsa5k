// DSA5000 High-Speed Client Implementation in Pure Rust
// Includes SSEP Multi-Module Protocol and Standalone Multi-Device Support (v2.01)

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::protocol::{
    parse_binary_packet, GlobalHeader, ModuleData, Packet, SsepModuleInfo, SsepSystemState,
    ValveMode, CHANNELS_PER_MODULE,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceEndpoint {
    pub host: String,
    pub cmd_port: u16,
    pub data_port: u16,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub ssep_role: crate::protocol::SsepAssignment,
}

impl DeviceEndpoint {
    pub fn new(host: &str, cmd_port: u16, data_port: u16) -> Self {
        Self {
            host: host.trim().to_string(),
            cmd_port,
            data_port,
            label: String::new(),
            ssep_role: crate::protocol::SsepAssignment::Auto,
        }
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self
    }

    pub fn with_role(mut self, role: crate::protocol::SsepAssignment) -> Self {
        self.ssep_role = role;
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct SystemInfo {
    pub model: String,
    pub serial_number: String,
    pub firmware_ver: String,
    pub valve_mode: String,
    pub scan_rate_hz: u32,
    pub engineering_units: String,
}

#[derive(Debug, Clone, Default)]
pub struct ClientStats {
    pub total_frames: u64,
    pub dropped_frames: u64,
    pub current_fps: u32,
    pub last_frame_number: u32,
    pub ptp_jitter_ns: i64,
    pub last_ptp_sec: u32,
    pub last_ptp_nsec: u32,
    pub bytes_received: u64,
}

/// A connection to a single DSA5000 physical or simulated unit
pub struct SingleDevice {
    pub endpoint: DeviceEndpoint,
    pub cmd_stream: Option<TcpStream>,
    pub data_stream: Option<TcpStream>,
    pub info: SystemInfo,
}

impl SingleDevice {
    pub fn new(endpoint: DeviceEndpoint) -> Self {
        Self {
            endpoint,
            cmd_stream: None,
            data_stream: None,
            info: SystemInfo {
                model: "Unknown".into(),
                serial_number: "Unknown".into(),
                firmware_ver: "Unknown".into(),
                valve_mode: "PX".into(),
                scan_rate_hz: 100,
                engineering_units: "PSI".into(),
            },
        }
    }

    pub fn connect(&mut self) -> io::Result<()> {
        // 1. Connect Command Socket (Port 23) with fallback to HTTP CGI
        let cmd_addr = format!("{}:{}", self.endpoint.host, self.endpoint.cmd_port);
        if let Ok(addrs) = cmd_addr.to_socket_addrs() {
            if let Some(addr) = addrs.into_iter().next() {
                if let Ok(mut cmd) = TcpStream::connect_timeout(&addr, Duration::from_millis(1000)) {
                    let _ = cmd.set_read_timeout(Some(Duration::from_millis(150)));
                    let mut banner_buf = [0u8; 1024];
                    let _ = cmd.read(&mut banner_buf);
                    let _ = cmd.set_read_timeout(Some(Duration::from_millis(200)));
                    let _ = cmd.set_write_timeout(Some(Duration::from_secs(1)));
                    self.cmd_stream = Some(cmd);
                } else {
                    self.cmd_stream = None;
                }
            }
        }

        // 2. Query initial instrument configuration
        let _ = self.query_system_info();

        // 3. Connect Binary Data Socket (Port 503)
        let data_addr = format!("{}:{}", self.endpoint.host, self.endpoint.data_port);
        if let Ok(addrs) = data_addr.to_socket_addrs() {
            if let Some(addr) = addrs.into_iter().next() {
                let data = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
                data.set_read_timeout(Some(Duration::from_millis(500)))?;
                data.set_write_timeout(Some(Duration::from_secs(2)))?;
                self.data_stream = Some(data);
            }
        }

        Ok(())
    }

    pub fn send_command(&mut self, cmd: &str) -> io::Result<String> {
        let clean_cmd = cmd.trim();
        if let Some(stream) = self.cmd_stream.as_mut() {
            let formatted = format!("{}\r\n", clean_cmd);
            if stream.write_all(formatted.as_bytes()).is_ok() && stream.flush().is_ok() {
                let mut buf = [0u8; 4096];
                let mut response = Vec::new();
                let start = Instant::now();

                // Wait up to 350ms for Telnet response
                while start.elapsed() < Duration::from_millis(350) {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            response.extend_from_slice(&buf[..n]);
                            if response.contains(&b'>') || response.ends_with(b"\r\n") {
                                break;
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                            if !response.is_empty() {
                                break;
                            }
                            thread::sleep(Duration::from_millis(15));
                        }
                        Err(e) => return Err(e),
                    }
                }

                let text = String::from_utf8_lossy(&response).to_string();
                if !text.trim().is_empty() {
                    return Ok(text);
                }

                // If physical device accepted setting command silently (e.g. "VALVE PX", "RATE 100", "FPS 0"),
                // quickly query the parameter to verify without freezing the GUI
                let upper = clean_cmd.to_uppercase();
                let query_cmd = if upper.starts_with("VALVE") || upper.starts_with("SET VALVE") {
                    Some("VALVE")
                } else if upper.starts_with("RATE") || upper.starts_with("SET RATE") {
                    Some("RATE")
                } else if upper.starts_with("FPS") || upper.starts_with("SET FPS") {
                    Some("FPS")
                } else if upper.starts_with("TRIG") || upper.starts_with("SET TRIG") {
                    Some("TRIG")
                } else if upper.starts_with("EU") || upper.starts_with("SET EU") {
                    Some("EU")
                } else if upper.starts_with("SSEP") {
                    Some("SSEP")
                } else {
                    None
                };

                if let Some(q) = query_cmd {
                    let q_fmt = format!("{}\r\n", q);
                    let _ = stream.write_all(q_fmt.as_bytes());
                    let _ = stream.flush();
                    let mut q_buf = [0u8; 1024];
                    let q_start = Instant::now();
                    let mut q_resp = Vec::new();
                    while q_start.elapsed() < Duration::from_millis(250) {
                        match stream.read(&mut q_buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                q_resp.extend_from_slice(&q_buf[..n]);
                                if q_resp.contains(&b'>') || q_resp.ends_with(b"\r\n") {
                                    break;
                                }
                            }
                            Err(_) => {
                                if !q_resp.is_empty() {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(10));
                            }
                        }
                    }
                    let q_text = String::from_utf8_lossy(&q_resp).to_string();
                    if !q_text.trim().is_empty() {
                        return Ok(q_text);
                    }
                }

                return Ok(format!("{}: OK", clean_cmd));
            }
        }

        // Fallback to HTTP CGI on Port 80
        self.send_http_cgi_command(cmd)
    }

    pub fn send_http_cgi_command(&self, cmd: &str) -> io::Result<String> {
        let clean_cmd = cmd.trim();
        let encoded_cmd = clean_cmd.replace(' ', "%20");
        let http_addr = format!("{}:80", self.endpoint.host);

        let addrs = http_addr.to_socket_addrs()?;
        let addr = addrs.into_iter().next().ok_or_else(|| {
            io::Error::new(io::ErrorKind::AddrNotAvailable, "Cannot resolve IP for HTTP CGI")
        })?;

        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))?;
        s.set_read_timeout(Some(Duration::from_millis(1500)))?;
        s.set_write_timeout(Some(Duration::from_millis(1500)))?;

        let req = format!(
            "GET /get.cgi?cmd={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            encoded_cmd, self.endpoint.host
        );
        s.write_all(req.as_bytes())?;
        s.flush()?;

        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let resp_str = String::from_utf8_lossy(&raw);

        if let Some(pos) = resp_str.find("\r\n\r\n") {
            Ok(resp_str[pos + 4..].to_string())
        } else {
            Ok(resp_str.to_string())
        }
    }

    pub fn query_system_info(&mut self) -> io::Result<()> {
        if let Ok(id_resp) = self.send_command("LIST ID") {
            for line in id_resp.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if parts[0] == "SN" {
                        self.info.serial_number = parts[1].to_string();
                    } else if parts[0] == "MODEL" {
                        self.info.model = parts[1].to_string();
                    }
                }
            }
        }

        if let Ok(s_resp) = self.send_command("LIST S") {
            for line in s_resp.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if parts[0] == "RATE" {
                        if let Ok(r) = parts[1].parse::<f32>() {
                            self.info.scan_rate_hz = r.round() as u32;
                        }
                    } else if parts[0] == "UNITS" {
                        self.info.engineering_units = parts[1].to_string();
                    }
                }
            }
        }

        if let Ok(v_resp) = self.send_command("VALVE") {
            for part in v_resp.split_whitespace() {
                if part == "PX" || part == "CAL" || part == "PRG" || part == "ISO" {
                    self.info.valve_mode = part.to_string();
                }
            }
        }

        if let Ok(ver) = self.send_command("VER") {
            let clean = ver.replace(['>', '\r', '\n'], "").trim().to_string();
            if !clean.is_empty() {
                self.info.firmware_ver = clean;
            }
        }

        if let Ok(sys) = self.send_command("LIST SYS") {
            for part in sys.split_whitespace() {
                if let Some(val) = part.strip_prefix("SN=") {
                    self.info.serial_number = val.to_string();
                } else if let Some(val) = part.strip_prefix("RATE=") {
                    if let Ok(r) = val.parse::<u32>() {
                        self.info.scan_rate_hz = r;
                    }
                } else if let Some(val) = part.strip_prefix("UNITS=") {
                    self.info.engineering_units = val.to_string();
                } else if let Some(val) = part.strip_prefix("VALVE=") {
                    self.info.valve_mode = val.to_string();
                }
            }
        }

        Ok(())
    }

    pub fn set_valve(&mut self, mode: ValveMode) -> io::Result<String> {
        let resp = self.send_command(&format!("VALVE {}", mode.as_str()))?;
        self.info.valve_mode = mode.as_str().to_string();
        Ok(resp)
    }

    pub fn set_rate(&mut self, hz: u32) -> io::Result<String> {
        let resp = self.send_command(&format!("RATE {}", hz))?;
        self.info.scan_rate_hz = hz;
        Ok(resp)
    }

    pub fn set_units(&mut self, unit: &str) -> io::Result<String> {
        let resp = self.send_command(&format!("EU {}", unit.to_uppercase()))?;
        self.info.engineering_units = unit.to_uppercase();
        Ok(resp)
    }

    pub fn set_trigger_mode(&mut self, mode: u32) -> io::Result<String> {
        self.send_command(&format!("TRIG {}", mode))
    }

    pub fn disconnect(&mut self) {
        if let Some(mut stream) = self.data_stream.take() {
            let _ = stream.write_all(&0u32.to_be_bytes());
            let _ = stream.flush();
        }
        if let Some(mut stream) = self.cmd_stream.take() {
            let _ = stream.write_all(b"STOP\r\n");
            let _ = stream.flush();
        }
    }
}

pub struct DSA5000Client {
    pub host: String,
    pub cmd_port: u16,
    pub data_port: u16,
    pub endpoints: Vec<DeviceEndpoint>,

    pub devices: Vec<SingleDevice>,

    is_connected: Arc<AtomicBool>,
    is_scanning: Arc<AtomicBool>,
    is_calibrating: Arc<AtomicBool>,

    pub system_info: Arc<Mutex<SystemInfo>>,
    pub ssep_state: Arc<Mutex<SsepSystemState>>,
    stats: Arc<Mutex<ClientStats>>,

    scan_threads: Vec<JoinHandle<()>>,
    stop_signal: Arc<AtomicBool>,
}

impl DSA5000Client {
    pub fn new(host: &str, cmd_port: u16, data_port: u16) -> Self {
        Self::new_multi(vec![DeviceEndpoint::new(host, cmd_port, data_port)])
    }

    pub fn new_multi(endpoints: Vec<DeviceEndpoint>) -> Self {
        let (first_host, first_cmd, first_data) = endpoints
            .first()
            .map(|e| (e.host.clone(), e.cmd_port, e.data_port))
            .unwrap_or_else(|| ("127.0.0.1".into(), 23, 503));

        Self {
            host: first_host,
            cmd_port: first_cmd,
            data_port: first_data,
            endpoints,
            devices: Vec::new(),
            is_connected: Arc::new(AtomicBool::new(false)),
            is_scanning: Arc::new(AtomicBool::new(false)),
            is_calibrating: Arc::new(AtomicBool::new(false)),
            system_info: Arc::new(Mutex::new(SystemInfo {
                model: "Unknown".into(),
                serial_number: "Unknown".into(),
                firmware_ver: "Unknown".into(),
                valve_mode: "PX".into(),
                scan_rate_hz: 100,
                engineering_units: "PSI".into(),
            })),
            ssep_state: Arc::new(Mutex::new(SsepSystemState::default())),
            stats: Arc::new(Mutex::new(ClientStats::default())),
            scan_threads: Vec::new(),
            stop_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    pub fn is_scanning(&self) -> bool {
        self.is_scanning.load(Ordering::SeqCst)
    }

    pub fn is_calibrating(&self) -> bool {
        self.is_calibrating.load(Ordering::SeqCst)
    }

    pub fn get_stats(&self) -> ClientStats {
        self.stats.lock().unwrap().clone()
    }

    pub fn connect(&mut self) -> io::Result<SystemInfo> {
        if self.is_connected() {
            self.disconnect();
        }

        self.devices.clear();
        let is_multi = self.endpoints.len() > 1;

        for ep in &self.endpoints {
            let mut dev = SingleDevice::new(ep.clone());
            dev.connect()?;
            let _ = dev.send_command("FPS 0");
            self.devices.push(dev);
        }

        if self.devices.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "No device endpoints configured"));
        }

        let primary_info = self.devices[0].info.clone();
        {
            let mut info_guard = self.system_info.lock().unwrap();
            *info_guard = primary_info.clone();
        }

        if !is_multi {
            // Single device mode: query SSEP or standalone single-device config
            let _ = self.query_ssep_system();
        } else {
            // Multi-device standalone mode: populate SSEP state with all connected standalone devices
            let mut state = SsepSystemState::default();
            state.unit_role = "Standalone (Multi-Device)".into();
            state.num_responders = self.devices.len() - 1;
            state.link_in = "N/A".into();
            state.link_out = "N/A".into();

            for (idx, dev) in self.devices.iter().enumerate() {
                let lbl = if !dev.endpoint.label.is_empty() {
                    dev.endpoint.label.clone()
                } else {
                    format!("STANDALONE-{}", idx)
                };
                let sn: u32 = dev.info.serial_number.parse().unwrap_or(4005 + idx as u32);
                state.modules.push(SsepModuleInfo {
                    index: idx,
                    model: dev.info.model.clone(),
                    serial_number: sn,
                    npr: "50.00".into(),
                    version: dev.info.firmware_ver.clone(),
                    rate_hz: dev.info.scan_rate_hz as f32,
                    valve_mode: dev.info.valve_mode.clone(),
                    units: dev.info.engineering_units.clone(),
                    ip: dev.endpoint.host.clone(),
                    label: lbl,
                });
            }
            let mut guard = self.ssep_state.lock().unwrap();
            *guard = state;
        }

        self.is_connected.store(true, Ordering::SeqCst);
        Ok(primary_info)
    }

    pub fn send_command_to(&mut self, target: Option<usize>, cmd: &str) -> io::Result<String> {
        match target {
            None => {
                if self.devices.is_empty() {
                    return Err(io::Error::new(io::ErrorKind::NotConnected, "No devices connected"));
                }
                let total_devs = self.devices.len();
                let mut outputs = Vec::new();
                for (i, dev) in self.devices.iter_mut().enumerate() {
                    match dev.send_command(cmd) {
                        Ok(resp) => {
                            if total_devs > 1 {
                                outputs.push(format!("[모듈 {} ({}): {}]", i, dev.endpoint.label, resp.trim()));
                            } else {
                                outputs.push(resp);
                            }
                        }
                        Err(e) => {
                            outputs.push(format!("[모듈 {} 에러: {}]", i, e));
                        }
                    }
                }
                Ok(outputs.join("\n"))
            }
            Some(idx) => {
                if let Some(dev) = self.devices.get_mut(idx) {
                    dev.send_command(cmd)
                } else {
                    Err(io::Error::new(io::ErrorKind::NotFound, format!("Module index {} not found", idx)))
                }
            }
        }
    }

    pub fn send_command(&mut self, cmd: &str) -> io::Result<String> {
        if self.devices.len() == 1 {
            self.send_command_to(Some(0), cmd)
        } else {
            self.send_command_to(None, cmd)
        }
    }

    pub fn query_ssep_system(&mut self) -> io::Result<SsepSystemState> {
        if self.endpoints.len() > 1 {
            let mut state = SsepSystemState::default();
            state.unit_role = "Standalone (Multi-Device)".into();
            state.num_responders = self.devices.len() - 1;
            state.link_in = "N/A".into();
            state.link_out = "N/A".into();

            for (idx, dev) in self.devices.iter_mut().enumerate() {
                let _ = dev.query_system_info();
                let lbl = if !dev.endpoint.label.is_empty() {
                    dev.endpoint.label.clone()
                } else {
                    format!("STANDALONE-{}", idx)
                };
                let sn: u32 = dev.info.serial_number.parse().unwrap_or(4005 + idx as u32);
                state.modules.push(SsepModuleInfo {
                    index: idx,
                    model: dev.info.model.clone(),
                    serial_number: sn,
                    npr: "50.00".into(),
                    version: dev.info.firmware_ver.clone(),
                    rate_hz: dev.info.scan_rate_hz as f32,
                    valve_mode: dev.info.valve_mode.clone(),
                    units: dev.info.engineering_units.clone(),
                    ip: dev.endpoint.host.clone(),
                    label: lbl,
                });
            }
            let mut ssep_guard = self.ssep_state.lock().unwrap();
            *ssep_guard = state.clone();
            return Ok(state);
        }

        let mut state = SsepSystemState::default();

        // 1. Query SMODE
        if let Ok(smode_resp) = self.send_command("SMODE") {
            for line in smode_resp.lines() {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix("Unit:") {
                    state.unit_role = rest.trim().to_string();
                } else if let Some(rest) = l.strip_prefix("Address:") {
                    state.address = rest.trim().parse().unwrap_or(0);
                } else if let Some(rest) = l.strip_prefix("Number of responders:") {
                    state.num_responders = rest.trim().parse().unwrap_or(0);
                } else if let Some(rest) = l.strip_prefix("LinkStatus:") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    for idx in 0..parts.len() {
                        if parts[idx] == "In" && idx + 1 < parts.len() {
                            state.link_in = parts[idx + 1].to_string();
                        } else if parts[idx] == "Out" && idx + 1 < parts.len() {
                            state.link_out = parts[idx + 1].to_string();
                        }
                    }
                }
            }
        }

        // 2. Query LIST SYS *
        if let Ok(list_resp) = self.send_command("LIST SYS *") {
            for line in list_resp.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 10 {
                    if let Ok(ndx) = parts[0].parse::<usize>() {
                        let mod_info = SsepModuleInfo {
                            index: ndx,
                            model: parts.get(1).unwrap_or(&"DSA5000").to_string(),
                            serial_number: parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(50000 + ndx as u32),
                            npr: parts.get(3).unwrap_or(&"15.00").to_string(),
                            version: parts.get(4).unwrap_or(&"2.01").to_string(),
                            rate_hz: parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(100.0),
                            valve_mode: parts.get(6).unwrap_or(&"PX").to_string(),
                            units: parts.get(7).unwrap_or(&"PSI").to_string(),
                            ip: parts.get(10).unwrap_or(&"191.30.130.1").to_string(),
                            label: parts.get(11).unwrap_or(&"MODULE").to_string(),
                        };
                        state.modules.push(mod_info);
                    }
                }
            }
        }

        // Fallback: If modules is empty but connected, insert at least controller
        if state.modules.is_empty() {
            let info = self.system_info.lock().unwrap().clone();
            state.modules.push(SsepModuleInfo {
                index: 0,
                model: info.model.clone(),
                serial_number: info.serial_number.parse().unwrap_or(50421),
                npr: "15.00".into(),
                version: "2.01".into(),
                rate_hz: info.scan_rate_hz as f32,
                valve_mode: info.valve_mode.clone(),
                units: info.engineering_units.clone(),
                ip: self.host.clone(),
                label: "CONTROLLER".into(),
            });
        }

        let mut guard = self.ssep_state.lock().unwrap();
        *guard = state.clone();
        Ok(state)
    }

    pub fn set_valve_target(&mut self, target: Option<usize>, mode: ValveMode) -> io::Result<String> {
        match target {
            None => {
                let mut count = 0;
                for dev in self.devices.iter_mut() {
                    let _ = dev.set_valve(mode);
                    count += 1;
                }
                let mut info = self.system_info.lock().unwrap();
                info.valve_mode = mode.as_str().to_string();
                Ok(format!("VALVE SET TO ALL {} DEVICES: {}", count, mode.as_str()))
            }
            Some(idx) => {
                if let Some(dev) = self.devices.get_mut(idx) {
                    let r = dev.set_valve(mode)?;
                    if idx == 0 {
                        let mut info = self.system_info.lock().unwrap();
                        info.valve_mode = mode.as_str().to_string();
                    }
                    Ok(format!("VALVE SET TO MODULE {}: {}", idx, r))
                } else {
                    Err(io::Error::new(io::ErrorKind::NotFound, format!("Module index {} not found", idx)))
                }
            }
        }
    }

    pub fn set_valve(&mut self, mode: ValveMode) -> io::Result<String> {
        self.set_valve_target(None, mode)
    }

    pub fn set_rate_target(&mut self, target: Option<usize>, hz: u32) -> io::Result<String> {
        match target {
            None => {
                for dev in &mut self.devices {
                    let _ = dev.set_rate(hz);
                }
                let mut info = self.system_info.lock().unwrap();
                info.scan_rate_hz = hz;
                Ok(format!("RATE SET TO {} HZ FOR ALL {} DEVICES", hz, self.devices.len()))
            }
            Some(idx) => {
                if let Some(dev) = self.devices.get_mut(idx) {
                    let r = dev.set_rate(hz)?;
                    if idx == 0 {
                        let mut info = self.system_info.lock().unwrap();
                        info.scan_rate_hz = hz;
                    }
                    Ok(format!("RATE SET TO {} HZ FOR MODULE {}: {}", hz, idx, r))
                } else {
                    Err(io::Error::new(io::ErrorKind::NotFound, format!("Module index {} not found", idx)))
                }
            }
        }
    }

    pub fn set_rate(&mut self, hz: u32) -> io::Result<String> {
        self.set_rate_target(None, hz)
    }

    pub fn set_units_target(&mut self, target: Option<usize>, unit: &str) -> io::Result<String> {
        match target {
            None => {
                for dev in &mut self.devices {
                    let _ = dev.set_units(unit);
                }
                let mut info = self.system_info.lock().unwrap();
                info.engineering_units = unit.to_uppercase();
                Ok(format!("UNITS SET TO {} FOR ALL {} DEVICES", unit, self.devices.len()))
            }
            Some(idx) => {
                if let Some(dev) = self.devices.get_mut(idx) {
                    let r = dev.set_units(unit)?;
                    if idx == 0 {
                        let mut info = self.system_info.lock().unwrap();
                        info.engineering_units = unit.to_uppercase();
                    }
                    Ok(format!("UNITS SET TO {} FOR MODULE {}: {}", unit, idx, r))
                } else {
                    Err(io::Error::new(io::ErrorKind::NotFound, format!("Module index {} not found", idx)))
                }
            }
        }
    }

    pub fn set_units(&mut self, unit: &str) -> io::Result<String> {
        self.set_units_target(None, unit)
    }

    pub fn set_trigger_mode_target(&mut self, target: Option<usize>, mode: u32) -> io::Result<String> {
        match target {
            None => {
                for dev in &mut self.devices {
                    let _ = dev.set_trigger_mode(mode);
                }
                Ok(format!("TRIGGER MODE SET TO {} FOR ALL {} DEVICES", mode, self.devices.len()))
            }
            Some(idx) => {
                if let Some(dev) = self.devices.get_mut(idx) {
                    let r = dev.set_trigger_mode(mode)?;
                    Ok(format!("TRIGGER MODE SET TO {} FOR MODULE {}: {}", mode, idx, r))
                } else {
                    Err(io::Error::new(io::ErrorKind::NotFound, format!("Module index {} not found", idx)))
                }
            }
        }
    }

    pub fn set_trigger_mode(&mut self, mode: u32) -> io::Result<String> {
        self.set_trigger_mode_target(None, mode)
    }

    pub fn set_fps_target(&mut self, target: Option<usize>, fps: u32) -> io::Result<String> {
        match target {
            None => {
                for dev in &mut self.devices {
                    let _ = dev.send_command(&format!("FPS {}", fps));
                }
                Ok(format!("FPS SET TO {} FOR ALL {} DEVICES", fps, self.devices.len()))
            }
            Some(idx) => {
                if let Some(dev) = self.devices.get_mut(idx) {
                    let r = dev.send_command(&format!("FPS {}", fps))?;
                    Ok(format!("FPS SET TO {} FOR MODULE {}: {}", fps, idx, r))
                } else {
                    Err(io::Error::new(io::ErrorKind::NotFound, format!("Module index {} not found", idx)))
                }
            }
        }
    }

    pub fn set_fps(&mut self, fps: u32) -> io::Result<String> {
        self.set_fps_target(None, fps)
    }

    pub fn get_device_valves(&self) -> Vec<String> {
        self.devices.iter().map(|d| d.info.valve_mode.clone()).collect()
    }

    pub fn set_ssep_mode(&mut self, target: Option<usize>, assignment: u8) -> io::Result<String> {
        let cmd = format!("SSEP {}", assignment);
        self.send_command_to(target, &cmd)
    }

    pub fn save_hardware(&mut self, target: Option<usize>) -> io::Result<String> {
        self.send_command_to(target, "SAVE")
    }

    pub fn get_ssep_mode(&mut self, target: Option<usize>) -> io::Result<String> {
        self.send_command_to(target, "SSEP")
    }

    pub fn zero_calibration_target<F>(&mut self, target: Option<usize>, mut progress_cb: F) -> io::Result<()>
    where
        F: FnMut(&str, u32),
    {
        if self.is_calibrating() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Calibration already active"));
        }

        let was_scanning = self.is_scanning();
        if was_scanning {
            self.stop_scan();
            thread::sleep(Duration::from_millis(200));
        }

        self.is_calibrating.store(true, Ordering::SeqCst);

        let target_indices: Vec<usize> = match target {
            None => (0..self.devices.len()).collect(),
            Some(idx) => {
                if idx < self.devices.len() {
                    vec![idx]
                } else {
                    self.is_calibrating.store(false, Ordering::SeqCst);
                    return Err(io::Error::new(io::ErrorKind::NotFound, format!("Module {} not found", idx)));
                }
            }
        };

        let target_desc = if let Some(idx) = target {
            format!("module {}", idx)
        } else {
            format!("all {} device(s)", self.devices.len())
        };

        progress_cb(&format!("Sending CALZ to {}...", target_desc), 10);

        for &i in &target_indices {
            if let Some(dev) = self.devices.get_mut(i) {
                let _ = dev.send_command("CALZ");
            }
        }
        progress_cb(&format!("Pneumatics cycling on {}...", target_desc), 40);

        let mut completed = false;
        let mut attempts = 0;
        while !completed && attempts < 20 {
            attempts += 1;
            thread::sleep(Duration::from_millis(500));
            let mut all_ready = true;
            for &i in &target_indices {
                if let Some(dev) = self.devices.get_mut(i) {
                    let status = dev.send_command("STATUS").unwrap_or_default();
                    if status.contains("BUSY") || status.contains("CALZ_ACTIVE") {
                        all_ready = false;
                        break;
                    }
                }
            }
            let pct = 40 + ((attempts * 50) / 20);
            progress_cb(&format!("Verifying zero offsets ({}/20)...", attempts), pct);
            if all_ready {
                completed = true;
            }
        }

        self.send_command_to(target, "SAVE")?;
        self.set_valve_target(target, ValveMode::Px)?;
        self.is_calibrating.store(false, Ordering::SeqCst);
        progress_cb(&format!("Zero Calibration for {} complete!", target_desc), 100);

        if was_scanning {
            let _ = self.start_scan(|_| {});
        }

        Ok(())
    }

    pub fn zero_calibration<F>(&mut self, progress_cb: F) -> io::Result<()>
    where
        F: FnMut(&str, u32),
    {
        self.zero_calibration_target(None, progress_cb)
    }

    pub fn start_scan<F>(&mut self, frame_callback: F) -> io::Result<()>
    where
        F: Fn(Packet) + Send + Sync + 'static,
    {
        if self.is_scanning() {
            return Ok(());
        }

        if self.devices.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "No devices to scan"));
        }

        // Trigger start (Big-Endian uint32 1) on all devices
        let mut read_streams = Vec::with_capacity(self.devices.len());
        for dev in &mut self.devices {
            let _ = dev.send_command("FPS 0");
            if let Some(stream) = dev.data_stream.as_mut() {
                stream.write_all(&1u32.to_be_bytes())?;
                stream.flush()?;
                let cloned = stream.try_clone()?;
                read_streams.push(cloned);
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!("Data stream not connected on {}", dev.endpoint.host),
                ));
            }
        }

        self.is_scanning.store(true, Ordering::SeqCst);
        self.stop_signal.store(false, Ordering::SeqCst);

        let callback_arc = Arc::new(frame_callback);
        let stop_signal = Arc::clone(&self.stop_signal);
        let is_scanning = Arc::clone(&self.is_scanning);
        let stats_arc = Arc::clone(&self.stats);
        let dev_count = read_streams.len();

        if dev_count == 1 {
            // Single device (SSEP or 1 Standalone)
            let mut read_stream = read_streams.remove(0);
            let handle = thread::spawn(move || {
                let mut read_buf = [0u8; 8192];
                let mut ring_buf = Vec::with_capacity(65536);
                let mut last_fps_calc = Instant::now();
                let mut frames_this_sec = 0u32;
                let mut prev_frame_num: Option<u32> = None;
                let mut prev_ptp_total_ns: Option<i64> = None;

                while !stop_signal.load(Ordering::SeqCst) {
                    match read_stream.read(&mut read_buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            ring_buf.extend_from_slice(&read_buf[..n]);

                            {
                                let mut st = stats_arc.lock().unwrap();
                                st.bytes_received += n as u64;
                            }

                            while !ring_buf.is_empty() {
                                match parse_binary_packet(&ring_buf) {
                                    Some((packet, consumed)) => {
                                        ring_buf.drain(..consumed);

                                        let mut dropped = 0u64;
                                        if let Some(prev) = prev_frame_num {
                                            if packet.header.frame_number > prev + 1 {
                                                dropped = (packet.header.frame_number - prev - 1) as u64;
                                            }
                                        }
                                        prev_frame_num = Some(packet.header.frame_number);

                                        let curr_ptp_ns = (packet.header.ptp_sec as i64 * 1_000_000_000)
                                            + packet.header.ptp_nsec as i64;
                                        let mut jitter = 0i64;
                                        if let Some(prev_ns) = prev_ptp_total_ns {
                                            let expected_interval_ns = 10_000_000;
                                            jitter = (curr_ptp_ns - prev_ns) - expected_interval_ns;
                                        }
                                        prev_ptp_total_ns = Some(curr_ptp_ns);

                                        frames_this_sec += 1;
                                        let elapsed = last_fps_calc.elapsed();
                                        let mut fps_update = None;
                                        if elapsed >= Duration::from_secs(1) {
                                            let fps = (frames_this_sec as f32 / elapsed.as_secs_f32()).round() as u32;
                                            fps_update = Some(fps);
                                            frames_this_sec = 0;
                                            last_fps_calc = Instant::now();
                                        }

                                        {
                                            let mut st = stats_arc.lock().unwrap();
                                            st.total_frames += 1;
                                            st.dropped_frames += dropped;
                                            st.last_frame_number = packet.header.frame_number;
                                            st.ptp_jitter_ns = jitter;
                                            st.last_ptp_sec = packet.header.ptp_sec;
                                            st.last_ptp_nsec = packet.header.ptp_nsec;
                                            if let Some(f) = fps_update {
                                                st.current_fps = f;
                                            }
                                        }

                                        callback_arc(packet);
                                    }
                                    None => break,
                                }
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                            thread::sleep(Duration::from_micros(500));
                        }
                        Err(_) => break,
                    }
                }

                let _ = read_stream.write_all(&0u32.to_be_bytes());
                let _ = read_stream.flush();
                is_scanning.store(false, Ordering::SeqCst);
            });

            self.scan_threads = vec![handle];
        } else {
            // Multi-device standalone: collect 1-module packets from each stream and aggregate into an N-module Packet
            let latest_modules = Arc::new(Mutex::new(vec![None; dev_count]));
            let mut handles = Vec::with_capacity(dev_count);

            for (dev_idx, mut read_stream) in read_streams.into_iter().enumerate() {
                let stop_sig = Arc::clone(&stop_signal);
                let latest_mods = Arc::clone(&latest_modules);
                let cb = Arc::clone(&callback_arc);
                let stats = Arc::clone(&stats_arc);
                let n_total = dev_count;

                let handle = thread::spawn(move || {
                    let mut read_buf = [0u8; 8192];
                    let mut ring_buf = Vec::with_capacity(65536);
                    let mut last_fps_calc = Instant::now();
                    let mut frames_this_sec = 0u32;
                    let mut prev_frame_num: Option<u32> = None;
                    let mut prev_ptp_total_ns: Option<i64> = None;

                    while !stop_sig.load(Ordering::SeqCst) {
                        match read_stream.read(&mut read_buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                ring_buf.extend_from_slice(&read_buf[..n]);
                                {
                                    let mut st = stats.lock().unwrap();
                                    st.bytes_received += n as u64;
                                }

                                while !ring_buf.is_empty() {
                                    match parse_binary_packet(&ring_buf) {
                                        Some((packet, consumed)) => {
                                            ring_buf.drain(..consumed);

                                            // If packet already contains multiple modules (SSEP Multi-Module Packet from Controller), emit directly
                                            if dev_idx == 0 && packet.modules.len() >= n_total {
                                                let mut dropped = 0u64;
                                                if let Some(prev) = prev_frame_num {
                                                    if packet.header.frame_number > prev + 1 {
                                                        dropped = (packet.header.frame_number - prev - 1) as u64;
                                                    }
                                                }
                                                prev_frame_num = Some(packet.header.frame_number);

                                                let curr_ptp_ns = (packet.header.ptp_sec as i64 * 1_000_000_000)
                                                    + packet.header.ptp_nsec as i64;
                                                let mut jitter = 0i64;
                                                if let Some(prev_ns) = prev_ptp_total_ns {
                                                    let expected_interval_ns = 10_000_000;
                                                    jitter = (curr_ptp_ns - prev_ns) - expected_interval_ns;
                                                }
                                                prev_ptp_total_ns = Some(curr_ptp_ns);

                                                frames_this_sec += 1;
                                                let elapsed = last_fps_calc.elapsed();
                                                let mut fps_update = None;
                                                if elapsed >= Duration::from_secs(1) {
                                                    let fps = (frames_this_sec as f32 / elapsed.as_secs_f32()).round() as u32;
                                                    fps_update = Some(fps);
                                                    frames_this_sec = 0;
                                                    last_fps_calc = Instant::now();
                                                }

                                                {
                                                    let mut st = stats.lock().unwrap();
                                                    st.total_frames += 1;
                                                    st.dropped_frames += dropped;
                                                    st.last_frame_number = packet.header.frame_number;
                                                    st.ptp_jitter_ns = jitter;
                                                    st.last_ptp_sec = packet.header.ptp_sec;
                                                    st.last_ptp_nsec = packet.header.ptp_nsec;
                                                    if let Some(f) = fps_update {
                                                        st.current_fps = f;
                                                    }
                                                }

                                                cb(packet);
                                                continue;
                                            }

                                            if let Some(m0) = packet.modules.into_iter().next() {
                                                let mut guard = latest_mods.lock().unwrap();
                                                guard[dev_idx] = Some(m0.clone());

                                                // Device 0 drives emission cadence (master clock)
                                                if dev_idx == 0 {
                                                    let mut all_modules = Vec::with_capacity(n_total);
                                                    for (k, opt) in guard.iter().enumerate() {
                                                        if let Some(m) = opt {
                                                            let mut m_clone = m.clone();
                                                            m_clone.responder_addr = k as u8;
                                                            all_modules.push(m_clone);
                                                        } else {
                                                            // Fallback if other device hasn't produced frame 1 yet
                                                            all_modules.push(ModuleData {
                                                                is_dsa: true,
                                                                serial_number: 4005 + k as u16,
                                                                responder_addr: k as u8,
                                                                is_scan_packet: true,
                                                                pressures: [0.0; CHANNELS_PER_MODULE],
                                                                temperatures: [25.0; CHANNELS_PER_MODULE],
                                                            });
                                                        }
                                                    }

                                                    let combined_packet = Packet {
                                                        header: GlobalHeader {
                                                            packet_id: packet.header.packet_id,
                                                            num_modules: n_total as u16,
                                                            frame_number: packet.header.frame_number,
                                                            ptp_sec: packet.header.ptp_sec,
                                                            ptp_nsec: packet.header.ptp_nsec,
                                                        },
                                                        modules: all_modules,
                                                        timestamp_ms: packet.timestamp_ms,
                                                    };

                                                    let mut dropped = 0u64;
                                                    if let Some(prev) = prev_frame_num {
                                                        if packet.header.frame_number > prev + 1 {
                                                            dropped = (packet.header.frame_number - prev - 1) as u64;
                                                        }
                                                    }
                                                    prev_frame_num = Some(packet.header.frame_number);

                                                    let curr_ptp_ns = (packet.header.ptp_sec as i64 * 1_000_000_000)
                                                        + packet.header.ptp_nsec as i64;
                                                    let mut jitter = 0i64;
                                                    if let Some(prev_ns) = prev_ptp_total_ns {
                                                        let expected_interval_ns = 10_000_000;
                                                        jitter = (curr_ptp_ns - prev_ns) - expected_interval_ns;
                                                    }
                                                    prev_ptp_total_ns = Some(curr_ptp_ns);

                                                    frames_this_sec += 1;
                                                    let elapsed = last_fps_calc.elapsed();
                                                    let mut fps_update = None;
                                                    if elapsed >= Duration::from_secs(1) {
                                                        let fps = (frames_this_sec as f32 / elapsed.as_secs_f32()).round() as u32;
                                                        fps_update = Some(fps);
                                                        frames_this_sec = 0;
                                                        last_fps_calc = Instant::now();
                                                    }

                                                    {
                                                        let mut st = stats.lock().unwrap();
                                                        st.total_frames += 1;
                                                        st.dropped_frames += dropped;
                                                        st.last_frame_number = packet.header.frame_number;
                                                        st.ptp_jitter_ns = jitter;
                                                        st.last_ptp_sec = packet.header.ptp_sec;
                                                        st.last_ptp_nsec = packet.header.ptp_nsec;
                                                        if let Some(f) = fps_update {
                                                            st.current_fps = f;
                                                        }
                                                    }

                                                    cb(combined_packet);
                                                }
                                            }
                                        }
                                        None => break,
                                    }
                                }
                            }
                            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                                thread::sleep(Duration::from_micros(500));
                            }
                            Err(_) => break,
                        }
                    }

                    let _ = read_stream.write_all(&0u32.to_be_bytes());
                    let _ = read_stream.flush();
                });

                handles.push(handle);
            }

            self.scan_threads = handles;
        }

        Ok(())
    }

    pub fn stop_scan(&mut self) {
        if !self.is_scanning() {
            return;
        }

        self.stop_signal.store(true, Ordering::SeqCst);

        for dev in &mut self.devices {
            if let Some(stream) = dev.data_stream.as_mut() {
                let _ = stream.write_all(&0u32.to_be_bytes());
                let _ = stream.flush();
            }
        }

        for h in self.scan_threads.drain(..) {
            let _ = h.join();
        }

        self.is_scanning.store(false, Ordering::SeqCst);
    }

    pub fn disconnect(&mut self) {
        if self.is_scanning() {
            self.stop_scan();
        }

        for dev in &mut self.devices {
            dev.disconnect();
        }
        self.devices.clear();

        self.is_connected.store(false, Ordering::SeqCst);
    }
}

impl Drop for DSA5000Client {
    fn drop(&mut self) {
        self.disconnect();
    }
}
