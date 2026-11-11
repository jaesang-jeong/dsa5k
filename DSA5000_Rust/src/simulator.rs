// Scanivalve DSA5000 Hardware & SSEP Multi-Module Simulator in Pure Rust
// Based on Scanivalve DSA5000 Manual V2.01 specifications

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::protocol::{
    CHANNELS_PER_MODULE, GLOBAL_HEADER_SIZE_STD, MODULE_DATA_SIZE, PACKET_ID,
};

pub struct DSA5000Simulator {
    pub cmd_port: u16,
    pub data_port: u16,
    pub num_modules: Arc<AtomicUsize>,
    pub is_standalone: Arc<AtomicBool>,
    pub base_sn: Arc<AtomicU32>,
    is_running: Arc<AtomicBool>,
    valve_mode: Arc<Mutex<String>>,
    rate_hz: Arc<AtomicU32>,
    cmd_handle: Option<JoinHandle<()>>,
    data_handle: Option<JoinHandle<()>>,
}

impl DSA5000Simulator {
    pub fn new(cmd_port: u16, data_port: u16) -> Self {
        Self {
            cmd_port,
            data_port,
            num_modules: Arc::new(AtomicUsize::new(2)), // Default: 2 modules (32 channels) in SSEP mode
            is_standalone: Arc::new(AtomicBool::new(false)),
            base_sn: Arc::new(AtomicU32::new(50421)),
            is_running: Arc::new(AtomicBool::new(false)),
            valve_mode: Arc::new(Mutex::new("PX".to_string())),
            rate_hz: Arc::new(AtomicU32::new(100)),
            cmd_handle: None,
            data_handle: None,
        }
    }

    pub fn set_base_sn(&self, sn: u32) {
        self.base_sn.store(sn, Ordering::SeqCst);
    }

    pub fn set_standalone(&self, standalone: bool) {
        self.is_standalone.store(standalone, Ordering::SeqCst);
        if standalone {
            self.num_modules.store(1, Ordering::SeqCst);
        }
    }

    pub fn set_module_count(&self, count: usize) {
        let clamped = count.clamp(1, 8);
        self.num_modules.store(clamped, Ordering::SeqCst);
    }

    pub fn start(&mut self) -> std::io::Result<()> {
        if self.is_running.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.is_running.store(true, Ordering::SeqCst);

        // 1. Telnet Command Server (Port 23 / 2323)
        let cmd_listener = TcpListener::bind(format!("0.0.0.0:{}", self.cmd_port))?;
        cmd_listener.set_nonblocking(true)?;
        let is_running_cmd = Arc::clone(&self.is_running);
        let valve_cmd = Arc::clone(&self.valve_mode);
        let rate_cmd = Arc::clone(&self.rate_hz);
        let num_mods_cmd = Arc::clone(&self.num_modules);
        let is_stand_cmd = Arc::clone(&self.is_standalone);
        let base_sn_cmd = Arc::clone(&self.base_sn);

        let cmd_handle = thread::spawn(move || {
            while is_running_cmd.load(Ordering::SeqCst) {
                match cmd_listener.accept() {
                    Ok((mut socket, _)) => {
                        let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
                        let welcome: &[u8] = if is_stand_cmd.load(Ordering::SeqCst) {
                            b"\r\n=== Scanivalve DSA5000 Standalone Simulator (Rust) ===\r\n>\r\n"
                        } else {
                            b"\r\n=== Scanivalve DSA5000 SSEP Simulator (Rust) ===\r\n>\r\n"
                        };
                        let _ = socket.write_all(welcome);

                        let mut buf = [0u8; 1024];
                        let mut line_buf = Vec::new();

                        while is_running_cmd.load(Ordering::SeqCst) {
                            match socket.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    line_buf.extend_from_slice(&buf[..n]);
                                    while let Some(pos) = line_buf.iter().position(|&b| b == b'\n') {
                                        let raw_line = line_buf.drain(..=pos).collect::<Vec<u8>>();
                                        let line = String::from_utf8_lossy(&raw_line).trim().to_uppercase();

                                        if !line.is_empty() {
                                            let resp = handle_sim_cmd(&line, &valve_cmd, &rate_cmd, &num_mods_cmd, &is_stand_cmd, &base_sn_cmd);
                                            let msg = format!("{}\r\n>\r\n", resp);
                                            if socket.write_all(msg.as_bytes()).is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                                    thread::sleep(Duration::from_millis(10));
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });

        // 2. Binary Streaming Server (Port 503 / 5503)
        let data_listener = TcpListener::bind(format!("0.0.0.0:{}", self.data_port))?;
        data_listener.set_nonblocking(true)?;
        let is_running_data = Arc::clone(&self.is_running);
        let valve_data = Arc::clone(&self.valve_mode);
        let rate_data = Arc::clone(&self.rate_hz);
        let num_mods_data = Arc::clone(&self.num_modules);
        let is_stand_data = Arc::clone(&self.is_standalone);
        let base_sn_data = Arc::clone(&self.base_sn);

        let data_handle = thread::spawn(move || {
            while is_running_data.load(Ordering::SeqCst) {
                match data_listener.accept() {
                    Ok((mut socket, _)) => {
                        let _ = socket.set_nonblocking(true);
                        let mut trigger_buf = [0u8; 4];
                        let mut is_streaming = false;
                        let mut frame_seq = 0u32;
                        let start_time = Instant::now();

                        while is_running_data.load(Ordering::SeqCst) {
                            // Non-blocking trigger check (1 = start, 0 = stop)
                            match socket.read_exact(&mut trigger_buf) {
                                Ok(()) => {
                                    let trigger = u32::from_be_bytes(trigger_buf);
                                    is_streaming = trigger == 1;
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                    // No trigger change requested, continue streaming state
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                                    break; // Client closed connection
                                }
                                Err(_) => {
                                    break; // Disconnected or reset
                                }
                            }

                            if is_streaming {
                                let rate = rate_data.load(Ordering::SeqCst).max(1);
                                let interval = Duration::from_micros((1_000_000 / rate) as u64);
                                let is_stand = is_stand_data.load(Ordering::SeqCst);
                                let n_mods = if is_stand { 1 } else { num_mods_data.load(Ordering::SeqCst) };
                                let base_sn_val = base_sn_data.load(Ordering::SeqCst);

                                let packet_bytes = generate_sim_packet(
                                    frame_seq,
                                    start_time.elapsed().as_secs_f32(),
                                    &valve_data.lock().unwrap(),
                                    n_mods,
                                    base_sn_val,
                                );
                                frame_seq += 1;

                                if socket.write_all(&packet_bytes).is_err() {
                                    break;
                                }
                                thread::sleep(interval);
                            } else {
                                thread::sleep(Duration::from_millis(10));
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });

        self.cmd_handle = Some(cmd_handle);
        self.data_handle = Some(data_handle);

        Ok(())
    }

    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        if let Some(h) = self.cmd_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.data_handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for DSA5000Simulator {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle_sim_cmd(
    line: &str,
    valve_mode: &Arc<Mutex<String>>,
    rate_hz: &Arc<AtomicU32>,
    num_modules: &Arc<AtomicUsize>,
    is_standalone: &Arc<AtomicBool>,
    base_sn: &Arc<AtomicU32>,
) -> String {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let op = parts.first().copied().unwrap_or("");
    let standalone = is_standalone.load(Ordering::SeqCst);
    let base_sn_val = base_sn.load(Ordering::SeqCst);

    match op {
        "MODEL" => {
            if standalone {
                "MODEL: DSA5000/16Pfx-15psid (Standalone Mode)".into()
            } else {
                "MODEL: DSA5000/16Pfx-15psid (SSEP Controller)".into()
            }
        }
        "STATUS" => {
            let v = valve_mode.lock().unwrap().clone();
            let r = rate_hz.load(Ordering::SeqCst);
            let n = if standalone { 1 } else { num_modules.load(Ordering::SeqCst) };
            if standalone {
                format!("STATUS: READY | VALVE: {} | RATE: {}Hz | MODE: STANDALONE (16 CH)", v, r)
            } else {
                format!("STATUS: READY | VALVE: {} | RATE: {}Hz | SSEP: CONTROLLER ({} MODS)", v, r, n)
            }
        }
        "SMODE" => {
            if standalone {
                "Unit: Standalone\r\nAddress: 0\r\nNumber of responders: 0\r\nLinkStatus: In DOWN Out DOWN".into()
            } else {
                let n = num_modules.load(Ordering::SeqCst);
                let responders = if n > 1 { (n - 1).to_string() } else { "0".to_string() };
                format!(
                    "Unit: Controller\r\nAddress: 0\r\nNumber of responders: {}\r\nLinkStatus: In DOWN Out UP",
                    responders
                )
            }
        }
        "LIST" => {
            let sub = parts.get(1).copied().unwrap_or("");
            let n = if standalone { 1 } else { num_modules.load(Ordering::SeqCst) };
            let v = valve_mode.lock().unwrap().clone();
            let r = rate_hz.load(Ordering::SeqCst);

            if sub == "SYS" {
                let has_star = parts.get(2).copied() == Some("*");
                if has_star {
                    let mut out = String::from(
                        "NDX -MODEL- -SN- -NPR- -VERSION- -RATE-- -VALVE- -UNITS- -CVTFACT- -ZC- ------IP------- --LABEL--\r\n"
                    );
                    for i in 0..n {
                        let sn = base_sn_val + i as u32;
                        let lbl = if standalone { "STANDALONE" } else if i == 0 { "CONTROLLER" } else { "RESPONDER" };
                        let ip = format!("191.30.130.{}", 21 + i);
                        out.push_str(&format!(
                            "{:2} DSA5000 {:5} 15.00 2.01 {:8.4} {} PSI 1.0000 1 {:17} {}-{}\r\n",
                            i, sn, r as f32, v, ip, lbl, i
                        ));
                    }
                    out.trim_end().to_string()
                } else {
                    let mut out = String::from("NDX -MODEL- --SN-- ------NPR------ --LABEL--\r\n");
                    for i in 0..n {
                        let sn = base_sn_val + i as u32;
                        let lbl = if standalone { "STANDALONE" } else if i == 0 { "CONTROLLER" } else { "RESPONDER" };
                        out.push_str(&format!("{:2} DSA5000 {:6} 15.00 {}-{}\r\n", i, sn, lbl, i));
                    }
                    out.trim_end().to_string()
                }
            } else if sub == "ID" {
                format!("SN {}\r\nMODEL DSA5000\r\nLABEL UNDEFINED\r\nNPR 50.0000 -15.0000\r\nXITE 1", base_sn_val)
            } else if sub == "S" {
                format!("RATE {}\r\nFPS 0\r\nFORMAT C\r\nUNITS PSI\r\nTRIG 0\r\nCALZ 1", r)
            } else if sub == "PTP" {
                "PTPEN 1\r\nSTAT 0\r\nSST 0:0:0.000000\r\nSSD 2026/1/1\r\nUTCO 0:00:00\r\nMAXOFM 50".into()
            } else {
                let mode_str = if standalone { "STANDALONE" } else { "SSEP" };
                format!("SYS CONFIG: SN={} MODEL=DSA5000/16Pfx RATE={} UNITS=PSI VALVE={} CHANNELS={} MODE={}", base_sn_val, r, v, n * 16, mode_str)
            }
        }
        "SSEP" => {
            let val = parts.get(1).copied().unwrap_or("");
            if val.is_empty() {
                if standalone {
                    "SSEP 1".into()
                } else {
                    "SSEP 0".into()
                }
            } else {
                match val {
                    "0" => {
                        is_standalone.store(false, Ordering::SeqCst);
                        "SSEP 0".into()
                    }
                    "1" => {
                        is_standalone.store(true, Ordering::SeqCst);
                        "SSEP 1".into()
                    }
                    "2" => {
                        is_standalone.store(false, Ordering::SeqCst);
                        "SSEP 2".into()
                    }
                    "3" => {
                        is_standalone.store(false, Ordering::SeqCst);
                        "SSEP 3".into()
                    }
                    "4" => {
                        is_standalone.store(false, Ordering::SeqCst);
                        "SSEP 4".into()
                    }
                    _ => format!("SSEP {}", val),
                }
            }
        }
        "SET" => {
            let param = parts.get(1).copied().unwrap_or("");
            let val = parts.get(2).copied().unwrap_or("");
            if param == "VALVE" {
                if ["PX", "RUN", "CAL", "PRG", "ISO", "OFF"].contains(&val) {
                    let mut v = valve_mode.lock().unwrap();
                    *v = (if val == "RUN" { "PX" } else { val }).to_string();
                    format!("VALVE SYNCHRONIZED TO ALL SSEP MODULES: {}", *v)
                } else {
                    format!("ERROR: Invalid valve mode '{}'", val)
                }
            } else if param == "RATE" {
                if let Ok(r) = val.parse::<u32>() {
                    let clamped = r.clamp(1, 5000);
                    rate_hz.store(clamped, Ordering::SeqCst);
                    format!("SSEP RATE SYNCHRONIZED TO {} HZ (Max Spec: 5000 Hz)", clamped)
                } else {
                    format!("ERROR: Invalid rate '{}'", val)
                }
            } else if param == "EU" || param == "UNITS" {
                format!("ENGINEERING UNITS SET TO {}", val)
            } else if param == "TRIG" {
                format!("TRIGGER MODE SET TO {}", val)
            } else if param == "FPS" {
                format!("FPS AVERAGE DECIMATION SET TO {}", val)
            } else if param == "AVG" {
                format!("FILTER MOVING AVERAGE SET TO {}", val)
            } else {
                format!("SET {} = {}", param, val)
            }
        }
        "RATE" => {
            let val = parts.get(1).copied().unwrap_or("");
            if val.is_empty() {
                format!("RATE {}", rate_hz.load(Ordering::SeqCst))
            } else if let Ok(r) = val.parse::<u32>() {
                let clamped = r.clamp(1, 5000);
                rate_hz.store(clamped, Ordering::SeqCst);
                format!("SSEP RATE SYNCHRONIZED TO {} HZ (Max Spec: 5000 Hz)", clamped)
            } else {
                format!("ERROR: Invalid rate '{}'", val)
            }
        }
        "EU" | "UNITS" => {
            let val = parts.get(1).copied().unwrap_or("PSI");
            format!("ENGINEERING UNITS SET TO {}", val)
        }
        "TRIG" => {
            let val = parts.get(1).copied().unwrap_or("0");
            format!("TRIGGER MODE SET TO {}", val)
        }
        "FPS" => {
            let val = parts.get(1).copied().unwrap_or("0");
            format!("FPS SET TO {}", val)
        }
        "VALVE" => {
            let val = parts.get(1).copied().unwrap_or("");
            if val.is_empty() {
                let v = valve_mode.lock().unwrap().clone();
                format!("VALVE {}", v)
            } else if ["PX", "RUN", "CAL", "PRG", "ISO", "OFF"].contains(&val) {
                let mut v = valve_mode.lock().unwrap();
                *v = (if val == "RUN" { "PX" } else { val }).to_string();
                format!("VALVE SYNCHRONIZED TO ALL SSEP MODULES: {}", *v)
            } else {
                format!("ERROR: Invalid valve mode '{}'", val)
            }
        }
        "CALZ" | "CAL" => "CALZ SYNCHRONOUSLY EXECUTED ON ALL SSEP MODULES (Complete)".into(),
        "CALB" => "CALB BAROMETRIC CALIBRATION EXECUTED (Complete)".into(),
        "SAVE" => "SETTINGS SAVED TO EEPROM (Controller & Responders)".into(),
        "RESET" => "SYSTEM REBOOTING... READY".into(),
        "VER" => "2.01 RTOS 2.00".into(),
        _ => format!("UNKNOWN COMMAND: '{}'", line),
    }
}

fn generate_sim_packet(frame_num: u32, t: f32, valve_mode: &str, num_modules: usize, base_sn: u32) -> Vec<u8> {
    let n_mods = num_modules.clamp(1, 8);
    let total_len = GLOBAL_HEADER_SIZE_STD + (n_mods * MODULE_DATA_SIZE);
    let mut buf = vec![0u8; total_len];

    // Global Header (28 bytes according to DSA5000 Manual Table 5.1)
    buf[0..2].copy_from_slice(&PACKET_ID.to_be_bytes());
    buf[2..4].copy_from_slice(&(n_mods as u16).to_be_bytes()); // Number of modules in SSEP packet
    buf[4..8].copy_from_slice(&frame_num.to_be_bytes());

    let now_dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let ptp_sec = now_dur.as_secs() as u32;
    let ptp_nsec = now_dur.subsec_nanos();
    buf[8..12].copy_from_slice(&ptp_sec.to_be_bytes());
    buf[12..16].copy_from_slice(&ptp_nsec.to_be_bytes());

    // Module Data Blocks (140 bytes each)
    let mut mod_offset = GLOBAL_HEADER_SIZE_STD;

    for m in 0..n_mods {
        let sn = (base_sn + m as u32) as u16;
        let mod_sn: u16 = 0x8000 | (sn & 0x7FFF); // Bit 15: 1=DSA
        buf[mod_offset..mod_offset + 2].copy_from_slice(&mod_sn.to_be_bytes());
        buf[mod_offset + 2] = m as u8; // Responder Address (0 for controller, 1..7 for responders)
        buf[mod_offset + 3] = 0x02; // Scan packet

        let press_offset = mod_offset + 12;
        for ch in 0..CHANNELS_PER_MODULE {
            let global_ch = (m * CHANNELS_PER_MODULE) + ch;
            let ch_f = global_ch as f32;

            let p = match valve_mode {
                "PX" => {
                    // Distinct frequency and phase for each module & channel
                    let base = 2.0 + ((ch_f % 16.0) * 0.35) + (m as f32 * 0.8);
                    let freq = 0.6 + (ch_f * 0.12);
                    let osc = ((2.0 * std::f32::consts::PI * freq * t) + (ch_f * 0.3)).sin()
                        * (0.18 + (ch_f * 0.015));
                    base + osc
                }
                "CAL" => 10.0 + (m as f32 * 0.05), // SSEP calibration level
                "PRG" => 45.0, // High-pressure purge
                "ISO" => 0.0, // Isolated
                _ => 0.0,
            };

            let p_bytes = p.to_be_bytes();
            let off = press_offset + (ch * 4);
            buf[off..off + 4].copy_from_slice(&p_bytes);
        }

        // Temperatures (~24.5 to 26.5 C)
        let temp_offset = press_offset + (CHANNELS_PER_MODULE * 4);
        for ch in 0..CHANNELS_PER_MODULE {
            let temp = 24.5 + (m as f32 * 0.5) + (ch as f32 * 0.08);
            let t_bytes = temp.to_be_bytes();
            let off = temp_offset + (ch * 4);
            buf[off..off + 4].copy_from_slice(&t_bytes);
        }

        mod_offset += MODULE_DATA_SIZE;
    }

    buf
}
