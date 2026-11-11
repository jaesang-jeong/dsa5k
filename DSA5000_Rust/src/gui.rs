// Scanivalve DSA5000 Native Desktop GUI Studio (SSEP Edition)
// Built in 100% Pure Rust with eframe & egui (Zero Web / Zero Browser Dependencies)

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use egui_plot::{Corner, HLine, Legend, Line, Plot, PlotPoints, VLine};
use serde::{Deserialize, Serialize};

use crate::client::{ClientStats, DSA5000Client, SystemInfo};
use crate::logger::CsvLogger;
use crate::protocol::{Packet, SsepSystemState, ValveMode, CHANNELS_PER_MODULE};
use crate::simulator::DSA5000Simulator;

const CHANNEL_COLORS: [Color32; 16] = [
    Color32::from_rgb(0, 229, 255),   // Cyan
    Color32::from_rgb(255, 61, 0),    // Orange-Red
    Color32::from_rgb(0, 230, 118),   // Green
    Color32::from_rgb(255, 234, 0),   // Yellow
    Color32::from_rgb(213, 0, 249),   // Magenta
    Color32::from_rgb(41, 121, 255),  // Blue
    Color32::from_rgb(255, 145, 0),   // Orange
    Color32::from_rgb(0, 176, 255),   // Light Blue
    Color32::from_rgb(118, 255, 3),   // Lime
    Color32::from_rgb(245, 0, 87),    // Pink
    Color32::from_rgb(29, 233, 182),  // Teal
    Color32::from_rgb(255, 109, 0),   // Deep Orange
    Color32::from_rgb(101, 31, 255),  // Purple
    Color32::from_rgb(198, 255, 0),   // Chartreuse
    Color32::from_rgb(255, 23, 68),   // Red
    Color32::from_rgb(0, 255, 200),   // Spring Green
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Oscilloscope,
    BarMeters,
    Statistics,
    SsepTopology,
    DataLogger,
    Terminal,
}

#[derive(Debug, Clone)]
pub struct ChannelStats {
    pub current: f32, // Calibrated active pressure = calc_current = raw * gain + offset
    pub raw_current: f32, // Raw unadjusted pressure from hardware packet
    pub sum: f64,
    pub sum_sq: f64,
    pub count: u64,
    pub min: f32,
    pub max: f32,
    pub temp: f32,
    pub calc_current: f32, // Scaled/calculated value = raw_current * gain + offset
}

impl Default for ChannelStats {
    fn default() -> Self {
        Self {
            current: 0.0,
            raw_current: 0.0,
            sum: 0.0,
            sum_sq: 0.0,
            count: 0,
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
            temp: 25.0,
            calc_current: 0.0,
        }
    }
}

impl ChannelStats {
    pub fn calc_value(&self, gain: f32, offset: f32) -> f32 {
        let base = if self.raw_current != 0.0 || self.current == 0.0 {
            self.raw_current
        } else {
            self.current
        };
        base * gain + offset
    }

    pub fn calc_mean(&self, gain: f32, offset: f32) -> f32 {
        if self.count > 0 {
            let raw_mean = (self.sum / self.count as f64) as f32;
            raw_mean * gain + offset
        } else {
            0.0
        }
    }

    pub fn calc_min(&self, gain: f32, offset: f32) -> f32 {
        if self.min.is_finite() && self.max.is_finite() {
            let v1 = self.min * gain + offset;
            let v2 = self.max * gain + offset;
            v1.min(v2)
        } else {
            0.0
        }
    }

    pub fn calc_max(&self, gain: f32, offset: f32) -> f32 {
        if self.min.is_finite() && self.max.is_finite() {
            let v1 = self.min * gain + offset;
            let v2 = self.max * gain + offset;
            v1.max(v2)
        } else {
            0.0
        }
    }

    pub fn calc_pp(&self, gain: f32) -> f32 {
        if self.min.is_finite() && self.max.is_finite() {
            (self.max - self.min).abs() * gain.abs()
        } else {
            0.0
        }
    }

    pub fn calc_rms(&self, gain: f32) -> f32 {
        if self.count > 1 {
            let mean = (self.sum / self.count as f64) as f32;
            let variance = ((self.sum_sq / self.count as f64) - (mean as f64 * mean as f64)).max(0.0);
            (variance.sqrt() as f32) * gain.abs()
        } else {
            0.0
        }
    }
}

/// Operating Mode: SSEP Multi-Module Synchronized or Standalone Single-Module
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperatingMode {
    Ssep,
    Standalone,
}

impl Default for OperatingMode {
    fn default() -> Self {
        OperatingMode::Ssep
    }
}

fn default_true() -> bool {
    true
}

fn default_line_width() -> f32 {
    1.0
}

/// Persisted application settings (saved to `dsa5000_config.json`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub host: String,
    pub cmd_port: u16,
    pub data_port: u16,
    pub is_simulator: bool,
    pub sim_module_count: usize,
    #[serde(default)]
    pub operating_mode: OperatingMode,
    pub scan_rate_hz: u32,
    pub engineering_units: String,
    pub trigger_mode: u32,
    pub fps_average: u32,
    pub time_window_sec: f64,
    pub auto_scale_y: bool,
    pub y_min: f64,
    pub y_max: f64,
    pub log_base_name: String,
    pub selected_modules: Vec<bool>,
    pub show_internal_legend: bool,
    pub bar_meter_cols: usize,
    pub bar_meter_max_psi: f32,
    #[serde(default)]
    pub standalone_devices: Vec<crate::client::DeviceEndpoint>,
    #[serde(default)]
    pub channel_names: Vec<String>,
    #[serde(default)]
    pub channel_gains: Vec<f32>,
    #[serde(default)]
    pub channel_offsets: Vec<f32>,
    #[serde(default = "default_true")]
    pub single_file_mode: bool,
    #[serde(default)]
    pub append_to_existing: bool,
    #[serde(default)]
    pub ssep_assignment: crate::protocol::SsepAssignment,
    #[serde(default = "default_line_width")]
    pub plot_line_width: f32,
    #[serde(default = "default_true")]
    pub show_cursor_info: bool,
    #[serde(default)]
    pub zoom_target_axis: ZoomTargetAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ZoomTargetAxis {
    All,
    X,
    Y,
}

impl Default for ZoomTargetAxis {
    fn default() -> Self {
        Self::All
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            cmd_port: 2323,
            data_port: 5503,
            is_simulator: true,
            sim_module_count: 2,
            operating_mode: OperatingMode::Ssep,
            scan_rate_hz: 100,
            engineering_units: "PSI".into(),
            trigger_mode: 0,
            fps_average: 0,
            time_window_sec: 5.0,
            auto_scale_y: true,
            y_min: 0.0,
            y_max: 15.0,
            log_base_name: "DSA5000_Measurement.csv".into(),
            selected_modules: vec![true; 8],
            show_internal_legend: false,
            bar_meter_cols: 8,
            bar_meter_max_psi: 15.0,
            standalone_devices: Vec::new(),
            channel_names: Vec::new(),
            channel_gains: vec![1.0; 128],
            channel_offsets: vec![0.0; 128],
            single_file_mode: true,
            append_to_existing: false,
            ssep_assignment: crate::protocol::SsepAssignment::Auto,
            plot_line_width: 1.0,
            show_cursor_info: true,
            zoom_target_axis: ZoomTargetAxis::All,
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        if let Ok(contents) = std::fs::read_to_string("dsa5000_config.json") {
            if let Ok(mut cfg) = serde_json::from_str::<AppConfig>(&contents) {
                if cfg.selected_modules.len() < 8 {
                    cfg.selected_modules.resize(8, true);
                }
                if cfg.channel_names.len() < 128 {
                    cfg.channel_names.resize(128, String::new());
                }
                if cfg.channel_gains.len() < 128 {
                    cfg.channel_gains.resize(128, 1.0);
                }
                if cfg.channel_offsets.len() < 128 {
                    cfg.channel_offsets.resize(128, 0.0);
                }
                return cfg;
            }
        }
        let mut cfg = Self::default();
        cfg.channel_names = vec![String::new(); 128];
        cfg.channel_gains = vec![1.0; 128];
        cfg.channel_offsets = vec![0.0; 128];
        cfg
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write("dsa5000_config.json", json);
        }
    }
}

pub struct DsaApp {
    // Network Settings
    pub host: String,
    pub cmd_port: u16,
    pub data_port: u16,
    pub is_simulator: bool,
    pub sim_module_count: usize,
    pub operating_mode: OperatingMode,
    pub ssep_assignment: crate::protocol::SsepAssignment,
    pub standalone_devices: Vec<crate::client::DeviceEndpoint>,
    pub show_device_manager_modal: bool,
    pub device_test_status: Vec<Option<bool>>,

    // Target Control (None = Simultaneous / All, Some(i) = Individual Module i)
    pub control_target: Option<usize>,
    pub module_valves: Vec<String>,
    pub scan_params_target: Option<usize>,
    pub terminal_target: Option<usize>,

    // Scan Parameters (DSA5000 Hardware Spec: Up to 5000 Hz)
    pub scan_rate_hz: u32,
    pub engineering_units: String,
    pub trigger_mode: u32,
    pub fps_average: u32,
    pub show_scan_params_modal: bool,

    // Core Backend Engine
    client: Option<DSA5000Client>,
    simulator: Option<DSA5000Simulator>,
    simulators: Vec<DSA5000Simulator>,
    logger: CsvLogger,

    // State Flags & Cached Info
    pub is_connected: bool,
    pub is_scanning: bool,
    pub is_calibrating: bool,
    pub system_info: SystemInfo,
    pub ssep_state: SsepSystemState,
    pub client_stats: ClientStats,

    // CALZ Progress Modal State
    pub calz_progress_pct: u32,
    pub calz_status_text: String,
    pub calz_start_time: Option<Instant>,
    pub calz_target: Option<usize>,
    pub calz_was_scanning: bool,

    // UI View Navigation
    pub selected_tab: AppTab,
    pub selected_modules: Vec<bool>, // Multi-module filter (up to 8 modules)
    pub show_internal_legend: bool,

    // Bar Meter Checkerboard Layout
    pub bar_meter_cols: usize, // 8, 16, or 4
    pub bar_meter_max_psi: f32, // Full scale limit

    // Visualization Buffer & Controls
    pub time_window_sec: f64,
    pub auto_scale_y: bool,
    pub is_paused: bool,
    pub y_min: f64,
    pub y_max: f64,
    pub plot_line_width: f32,
    pub y_zoom_factor: f64,
    pub show_cursor_info: bool,
    pub cursor_pos: Option<(f64, f64)>,
    pub zoom_target_axis: ZoomTargetAxis,
    pub first_ptp_time: Option<f64>,
    pub is_command_pending: bool,
    pub command_pending_desc: String,
    pub command_pending_start: Option<Instant>,
    pub active_channels: Vec<bool>,
    pub data_history: Vec<(f64, Vec<f32>)>, // (time_sec, pressures for all channels)
    pub channel_stats: Vec<ChannelStats>,

    // Channel Naming, Gain/Offset Calibration & Identification
    pub channel_names: Vec<String>,
    pub channel_gains: Vec<f32>,
    pub channel_offsets: Vec<f32>,
    pub batch_gain: f32,
    pub batch_offset: f32,
    pub show_channel_names_modal: bool,
    pub channel_name_prefix: String,
    pub channel_name_selected_mod: usize,

    // Inter-thread Packet Receiver
    frame_rx: Receiver<Packet>,
    frame_tx: Sender<Packet>,

    // Terminal Emulator
    pub terminal_input: String,
    pub terminal_history: Vec<(String, Color32)>,

    // CSV Logger UI
    pub log_base_name: String,
    pub last_logged_path: Option<String>,
    pub single_file_mode: bool,
    pub append_to_existing: bool,

    // Live PTP Hardware Timing
    pub last_ptp_sec: u32,
    pub last_ptp_nsec: u32,

    start_instant: Instant,
}

#[cfg(windows)]
pub fn set_win32_busy_cursor(busy: bool) {
    #[link(name = "user32")]
    extern "system" {
        fn LoadCursorW(hInstance: *const std::ffi::c_void, lpCursorName: *const u16) -> *mut std::ffi::c_void;
        fn SetCursor(hCursor: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    unsafe {
        let cursor_id = if busy {
            // IDC_WAIT = 32514 (Standard Windows Busy / Wait Spinning Ring)
            32514 as usize as *const u16
        } else {
            // IDC_ARROW = 32512 (Normal Pointer)
            32512 as usize as *const u16
        };
        let h_cursor = LoadCursorW(std::ptr::null(), cursor_id);
        if !h_cursor.is_null() {
            SetCursor(h_cursor);
        }
    }
}

#[cfg(not(windows))]
pub fn set_win32_busy_cursor(_busy: bool) {}

impl DsaApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Register Korean font (Malgun Gothic) to fix broken Korean glyphs
        Self::setup_custom_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let cfg = AppConfig::load();
        let (tx, rx) = channel();

        // 128 channels max (8 modules x 16 channels)
        let active_channels = vec![true; 128];
        let channel_stats = vec![ChannelStats::default(); 128];

        let channel_names = if cfg.channel_names.len() >= 128 {
            cfg.channel_names
        } else {
            let mut names = cfg.channel_names;
            names.resize(128, String::new());
            names
        };

        let channel_gains = if cfg.channel_gains.len() >= 128 {
            cfg.channel_gains
        } else {
            let mut g = cfg.channel_gains;
            g.resize(128, 1.0);
            g
        };

        let channel_offsets = if cfg.channel_offsets.len() >= 128 {
            cfg.channel_offsets
        } else {
            let mut o = cfg.channel_offsets;
            o.resize(128, 0.0);
            o
        };

        let app = Self {
            host: cfg.host.clone(),
            cmd_port: cfg.cmd_port,
            data_port: cfg.data_port,
            is_simulator: cfg.is_simulator,
            sim_module_count: cfg.sim_module_count,
            operating_mode: cfg.operating_mode,
            ssep_assignment: cfg.ssep_assignment,

            scan_rate_hz: cfg.scan_rate_hz,
            engineering_units: cfg.engineering_units,
            trigger_mode: cfg.trigger_mode,
            fps_average: cfg.fps_average,
            show_scan_params_modal: false,

            client: None,
            simulator: None,
            simulators: Vec::new(),
            logger: CsvLogger::new("./data_records"),
            standalone_devices: if cfg.standalone_devices.is_empty() {
                vec![crate::client::DeviceEndpoint::new(&cfg.host, cfg.cmd_port, cfg.data_port)]
            } else {
                cfg.standalone_devices
            },
            show_device_manager_modal: false,
            device_test_status: Vec::new(),

            control_target: None,
            module_valves: vec!["PX".into(); 8],
            scan_params_target: None,
            terminal_target: None,

            is_connected: false,
            is_scanning: false,
            is_calibrating: false,
            system_info: SystemInfo::default(),
            ssep_state: SsepSystemState::default(),
            client_stats: ClientStats::default(),

            calz_progress_pct: 0,
            calz_status_text: String::new(),
            calz_start_time: None,
            calz_target: None,
            calz_was_scanning: false,

            selected_tab: AppTab::Oscilloscope,
            selected_modules: cfg.selected_modules,
            show_internal_legend: cfg.show_internal_legend,

            bar_meter_cols: cfg.bar_meter_cols,
            bar_meter_max_psi: cfg.bar_meter_max_psi,

            time_window_sec: cfg.time_window_sec,
            auto_scale_y: cfg.auto_scale_y,
            is_paused: false,
            y_min: cfg.y_min,
            y_max: cfg.y_max,
            plot_line_width: cfg.plot_line_width,
            y_zoom_factor: 1.0,
            show_cursor_info: cfg.show_cursor_info,
            cursor_pos: None,
            zoom_target_axis: cfg.zoom_target_axis,
            first_ptp_time: None,
            is_command_pending: false,
            command_pending_desc: String::new(),
            command_pending_start: None,
            active_channels,
            data_history: Vec::with_capacity(65536),
            channel_stats,

            channel_names,
            channel_gains,
            channel_offsets,
            batch_gain: 1.0,
            batch_offset: 0.0,
            show_channel_names_modal: false,
            channel_name_prefix: "Ch_".to_string(),
            channel_name_selected_mod: 99,

            frame_rx: rx,
            frame_tx: tx,

            terminal_input: String::new(),
            terminal_history: vec![
                (
                    "DSA5000 Studio (SSEP Native Edition v2.01) Initialized.".into(),
                    Color32::from_rgb(0, 229, 255),
                ),
                (
                    "Ready to connect. Check 'Simulator Mode' for local SSEP testing, or enter physical DSA IP.".into(),
                    Color32::LIGHT_GRAY,
                ),
                (
                    "Loaded persistent settings from dsa5000_config.json.".into(),
                    Color32::from_rgb(170, 255, 170),
                ),
            ],

            log_base_name: cfg.log_base_name,
            last_logged_path: None,
            single_file_mode: cfg.single_file_mode,
            append_to_existing: cfg.append_to_existing,

            last_ptp_sec: 0,
            last_ptp_nsec: 0,

            start_instant: Instant::now(),
        };

        app
    }

    pub fn save_config(&self) {
        let cfg = AppConfig {
            host: self.host.clone(),
            cmd_port: self.cmd_port,
            data_port: self.data_port,
            is_simulator: self.is_simulator,
            sim_module_count: self.sim_module_count,
            operating_mode: self.operating_mode,
            ssep_assignment: self.ssep_assignment,
            standalone_devices: self.standalone_devices.clone(),
            scan_rate_hz: self.scan_rate_hz,
            engineering_units: self.engineering_units.clone(),
            trigger_mode: self.trigger_mode,
            fps_average: self.fps_average,
            time_window_sec: self.time_window_sec,
            auto_scale_y: self.auto_scale_y,
            y_min: self.y_min,
            y_max: self.y_max,
            log_base_name: self.log_base_name.clone(),
            selected_modules: self.selected_modules.clone(),
            show_internal_legend: self.show_internal_legend,
            bar_meter_cols: self.bar_meter_cols,
            bar_meter_max_psi: self.bar_meter_max_psi,
            channel_names: self.channel_names.clone(),
            channel_gains: self.channel_gains.clone(),
            channel_offsets: self.channel_offsets.clone(),
            single_file_mode: self.single_file_mode,
            append_to_existing: self.append_to_existing,
            plot_line_width: self.plot_line_width,
            show_cursor_info: self.show_cursor_info,
            zoom_target_axis: self.zoom_target_axis,
        };
        cfg.save();
    }

    pub fn switch_operating_mode(&mut self, new_mode: OperatingMode) {
        if self.operating_mode == new_mode {
            return;
        }

        self.operating_mode = new_mode;

        // If connected, synchronize hardware setting via SSEP command
        if self.is_connected {
            if let Some(c) = self.client.as_mut() {
                if new_mode == OperatingMode::Standalone {
                    match c.set_ssep_mode(None, 1) {
                        Ok(resp) => {
                            self.terminal_history.push((
                                format!("[SSEP MODE] 스탠드얼론 강제 설정 완료 (SSEP 1): {}", resp.trim()),
                                Color32::from_rgb(0, 229, 255),
                            ));
                        }
                        Err(e) => {
                            self.terminal_history.push((
                                format!("[ERROR] 스탠드얼론 SSEP 1 설정 실패: {}", e),
                                Color32::from_rgb(255, 60, 60),
                            ));
                        }
                    }
                } else {
                    // SSEP Mode
                    if c.devices.len() > 1 {
                        for (i, dev) in c.devices.iter_mut().enumerate() {
                            let role_val = if let Some(ed) = self.standalone_devices.get(i) {
                                if ed.ssep_role != crate::protocol::SsepAssignment::Auto {
                                    ed.ssep_role.to_u8()
                                } else if i == 0 {
                                    2 // Controller
                                } else if i == self.standalone_devices.len() - 1 {
                                    4 // Last Responder
                                } else {
                                    3 // Middle Responder
                                }
                            } else if i == 0 {
                                2
                            } else {
                                3
                            };
                            let _ = dev.send_command(&format!("SSEP {}", role_val));
                        }
                        self.terminal_history.push((
                            "[SSEP MODE] 다중 모듈 SSEP 역할 분배 완료 (Mod 0: Controller, Mod 1..N: Responder)".into(),
                            Color32::from_rgb(0, 229, 255),
                        ));
                    } else {
                        let ssep_val = self.ssep_assignment.to_u8();
                        match c.set_ssep_mode(None, ssep_val) {
                            Ok(resp) => {
                                self.terminal_history.push((
                                    format!("[SSEP MODE] 하드웨어 SSEP 설정 완료: SSEP {} ({}) - {}", ssep_val, self.ssep_assignment.label(), resp.trim()),
                                    Color32::from_rgb(0, 229, 255),
                                ));
                            }
                            Err(e) => {
                                self.terminal_history.push((
                                    format!("[ERROR] SSEP 모드 설정 실패: {}", e),
                                    Color32::from_rgb(255, 60, 60),
                                ));
                            }
                        }
                    }

                    if let Ok(st) = c.query_ssep_system() {
                        self.ssep_state = st;
                    }
                }
            }

            // Sync running simulator instances if in simulation
            if self.is_simulator {
                let is_stand = new_mode == OperatingMode::Standalone;
                if let Some(s) = self.simulator.as_ref() {
                    s.set_standalone(is_stand);
                }
                for s in &self.simulators {
                    s.set_standalone(is_stand);
                }
            }
        }

        self.save_config();
    }

    pub fn current_module_count(&self) -> usize {
        if self.is_connected {
            self.ssep_state.modules.len().max(1)
        } else if self.operating_mode == OperatingMode::Standalone {
            if self.is_simulator {
                self.sim_module_count
            } else {
                self.standalone_devices.len().max(1)
            }
        } else {
            self.sim_module_count
        }
    }

    pub fn sync_host_string_from_devices(&mut self) {
        if self.standalone_devices.is_empty() {
            self.standalone_devices.push(crate::client::DeviceEndpoint::new(&self.host, self.cmd_port, self.data_port));
        }
        let hosts: Vec<String> = self.standalone_devices.iter().map(|d| d.host.clone()).collect();
        self.host = hosts.join(", ");
    }

    pub fn sync_devices_from_host_string(&mut self) {
        let parts: Vec<&str> = self.host.split([',', ';']).map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return;
        }
        let mut new_devices = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            let (cmd, data, label, ssep_role) = if let Some(existing) = self.standalone_devices.get(i) {
                (existing.cmd_port, existing.data_port, existing.label.clone(), existing.ssep_role)
            } else {
                let default_role = if i == 0 {
                    crate::protocol::SsepAssignment::Controller
                } else {
                    crate::protocol::SsepAssignment::MiddleResponder
                };
                (self.cmd_port, self.data_port, format!("장치 {} (Ch {}-{})", i + 1, i * 16 + 1, (i + 1) * 16), default_role)
            };
            new_devices.push(crate::client::DeviceEndpoint {
                host: p.to_string(),
                cmd_port: cmd,
                data_port: data,
                label,
                ssep_role,
            });
        }
        self.standalone_devices = new_devices;
    }

    pub fn get_channel_custom_name(&self, global_ch: usize) -> &str {
        self.channel_names.get(global_ch).map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("")
    }

    fn setup_custom_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();

        let mut font_candidates = vec![
            "C:\\Windows\\Fonts\\malgun.ttf".to_string(),       // 맑은 고딕 (Malgun Gothic)
            "C:\\Windows\\Fonts\\malgunbd.ttf".to_string(),     // 맑은 고딕 Bold
            "C:\\Windows\\Fonts\\NanumGothic.ttf".to_string(),  // 나눔고딕
            "C:\\Windows\\Fonts\\gulim.ttc".to_string(),        // 굴림
            "C:\\Windows\\Fonts\\batang.ttc".to_string(),       // 바탕
        ];

        // Also check %WINDIR%\Fonts if WINDIR environment variable exists
        if let Ok(windir) = std::env::var("WINDIR") {
            font_candidates.insert(0, format!("{}\\Fonts\\malgun.ttf", windir));
            font_candidates.insert(1, format!("{}\\Fonts\\malgunbd.ttf", windir));
        }

        let mut loaded_korean = false;
        for path in &font_candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts.font_data.insert(
                    "korean_font".to_owned(),
                    egui::FontData::from_owned(bytes),
                );
                loaded_korean = true;
                break;
            }
        }

        // Also check for Segoe UI Emoji for emoji icons
        if let Ok(emoji_bytes) = std::fs::read("C:\\Windows\\Fonts\\seguiemj.ttf") {
            fonts.font_data.insert(
                "seguiemj".to_owned(),
                egui::FontData::from_owned(emoji_bytes),
            );
        }

        if loaded_korean {
            // Set as primary font for proportional text (UI labels, buttons, tabs)
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "korean_font".to_owned());
                if fonts.font_data.contains_key("seguiemj") {
                    family.push("seguiemj".to_owned());
                }
            }

            // Set as fallback for monospace text (terminal logs, tables, values)
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.push("korean_font".to_owned());
                if fonts.font_data.contains_key("seguiemj") {
                    family.push("seguiemj".to_owned());
                }
            }

            ctx.set_fonts(fonts);
        }
    }

    pub fn handle_connect(&mut self) {
        self.trigger_command_pending("장치 연결 및 초기화 중...");
        let is_stand = self.operating_mode == OperatingMode::Standalone;

        // Clean up any previously running simulator instances
        for mut sim in self.simulators.drain(..) {
            sim.stop();
        }
        if let Some(mut sim) = self.simulator.take() {
            sim.stop();
        }

        let connect_endpoints: Vec<crate::client::DeviceEndpoint> = if self.is_simulator {
            if is_stand {
                let dev_count = self.sim_module_count.clamp(1, 8);
                self.terminal_history.push((
                    format!("[*] Starting {} Standalone Hardware Simulator instances (16 channels each, total {} Ch)...", dev_count, dev_count * 16),
                    Color32::from_rgb(255, 234, 0),
                ));
                let mut eps = Vec::with_capacity(dev_count);
                for i in 0..dev_count {
                    let cp = self.cmd_port + i as u16;
                    let dp = self.data_port + i as u16;
                    let mut sim = DSA5000Simulator::new(cp, dp);
                    sim.set_standalone(true);
                    sim.set_base_sn(50421 + i as u32);
                    sim.set_module_count(1);
                    if let Err(e) = sim.start() {
                        self.terminal_history.push((
                            format!("[ERROR] Simulator #{} start failed on ports {}/{}: {}", i, cp, dp, e),
                            Color32::from_rgb(255, 23, 68),
                        ));
                        return;
                    }
                    self.simulators.push(sim);
                    eps.push(crate::client::DeviceEndpoint::new("127.0.0.1", cp, dp).with_label(&format!("Sim_Dev_{}", i + 1)));
                }
                thread::sleep(Duration::from_millis(150));
                eps
            } else {
                let mod_cnt = self.sim_module_count;
                self.terminal_history.push((
                    format!("[*] Starting multi-module SSEP Hardware Simulator ({} modules, {} channels) on ports {}/{}...", mod_cnt, mod_cnt * 16, self.cmd_port, self.data_port),
                    Color32::from_rgb(255, 234, 0),
                ));
                let mut sim = DSA5000Simulator::new(self.cmd_port, self.data_port);
                sim.set_standalone(false);
                sim.set_module_count(mod_cnt);
                if let Err(e) = sim.start() {
                    self.terminal_history.push((
                        format!("[ERROR] Simulator start failed: {}", e),
                        Color32::from_rgb(255, 23, 68),
                    ));
                    return;
                }
                self.simulator = Some(sim);
                thread::sleep(Duration::from_millis(150));
                vec![crate::client::DeviceEndpoint::new(&self.host, self.cmd_port, self.data_port)]
            }
        } else {
            self.sync_devices_from_host_string();
            if self.standalone_devices.is_empty() {
                vec![crate::client::DeviceEndpoint::new(&self.host, self.cmd_port, self.data_port)]
            } else {
                self.standalone_devices.clone()
            }
        };

        let target_str = if is_stand {
            if connect_endpoints.len() > 1 {
                format!("{} Standalone Units (Concurrent)", connect_endpoints.len())
            } else {
                "DSA5000 Standalone Unit".to_string()
            }
        } else {
            if connect_endpoints.len() > 1 {
                format!("{} SSEP Daisy Chain Units", connect_endpoints.len())
            } else {
                "SSEP Controller".to_string()
            }
        };
        self.terminal_history.push((
            format!("[*] Connecting to {} ({:?})...", target_str, connect_endpoints.iter().map(|e| &e.host).collect::<Vec<_>>()),
            Color32::from_rgb(0, 230, 118),
        ));

        let mut client = DSA5000Client::new_multi(connect_endpoints);
        match client.connect() {
            Ok(info) => {
                self.system_info = info.clone();
                let ssep = client.ssep_state.lock().unwrap().clone();
                self.ssep_state = ssep;

                let num_mods = self.ssep_state.modules.len();
                for i in 0..num_mods.min(self.selected_modules.len()) {
                    self.selected_modules[i] = true;
                }

                let cur_cnt = self.current_module_count();
                self.module_valves = client.get_device_valves();
                if self.module_valves.len() < cur_cnt {
                    self.module_valves.resize(cur_cnt, "PX".into());
                }

                // Sync saved scan parameters to hardware/simulator
                let _ = client.set_rate(self.scan_rate_hz);
                let _ = client.set_units(&self.engineering_units);
                let _ = client.set_trigger_mode(self.trigger_mode);

                // Check and report active hardware SSEP mode across connected modules
                for (i, dev) in self.standalone_devices.iter().enumerate() {
                    if let Ok(cur_ssep) = client.get_ssep_mode(Some(i)) {
                        self.terminal_history.push((
                            format!("[SSEP] 모듈 {} ({}) 현재 하드웨어 역할: {}", i, dev.host, cur_ssep.trim()),
                            Color32::from_rgb(0, 229, 255),
                        ));
                    }
                }

                self.is_connected = true;
                let connect_log = if is_stand {
                    format!(
                        "[+] Connected! Standalone Mode: {} device(s) (Total {} Channels). Primary Model: {} (S/N: {}). Rate: {} Hz",
                        num_mods,
                        num_mods * 16,
                        info.model,
                        info.serial_number,
                        self.scan_rate_hz
                    )
                } else {
                    format!(
                        "[+] Connected! Controller: {} (S/N: {}). SSEP Role: {}. Discovered Modules: {}. Rate: {} Hz",
                        info.model,
                        info.serial_number,
                        self.ssep_state.unit_role,
                        self.ssep_state.modules.len(),
                        self.scan_rate_hz
                    )
                };
                self.terminal_history.push((connect_log, Color32::from_rgb(0, 230, 118)));
                self.terminal_history.push((
                    "💡 계측 준비 완료. 상단 리본의 [▶ 데이터 수집 시작 (SCAN)] 버튼을 클릭하여 스캔을 시작하세요.".into(),
                    Color32::from_rgb(255, 234, 0),
                ));
                self.client = Some(client);
                self.save_config();
            }
            Err(e) => {
                self.terminal_history.push((
                    format!("[ERROR] Connection failed: {}", e),
                    Color32::from_rgb(255, 23, 68),
                ));
                if let Some(mut sim) = self.simulator.take() {
                    sim.stop();
                }
                for mut sim in self.simulators.drain(..) {
                    sim.stop();
                }
            }
        }
        self.finish_command_pending();
    }

    pub fn trigger_command_pending(&mut self, desc: impl Into<String>) {
        self.is_command_pending = true;
        self.command_pending_desc = desc.into();
        self.command_pending_start = Some(Instant::now());
        set_win32_busy_cursor(true);
    }

    pub fn finish_command_pending(&mut self) {
        self.command_pending_start = Some(Instant::now());
    }

    pub fn handle_disconnect(&mut self) {
        if self.is_scanning {
            self.handle_stop_scan();
        }
        if let Some(mut c) = self.client.take() {
            c.disconnect();
        }
        if let Some(mut sim) = self.simulator.take() {
            sim.stop();
        }
        for mut sim in self.simulators.drain(..) {
            sim.stop();
        }
        self.is_connected = false;
        self.save_config();
        self.terminal_history.push((
            "[-] Disconnected and all sockets safely closed (0 sent to Port 503).".into(),
            Color32::LIGHT_GRAY,
        ));
    }

    pub fn handle_start_scan(&mut self) {
        if let Some(c) = self.client.as_mut() {
            let tx = self.frame_tx.clone();
            self.first_ptp_time = None;
            self.data_history.clear();
            self.cursor_pos = None;
            match c.start_scan(move |pkt| {
                let _ = tx.send(pkt);
            }) {
                Ok(_) => {
                    self.is_scanning = true;
                    self.terminal_history.push((
                        format!("▶ SSEP Synchronous Scan started at {} Hz (Port 503 binary stream active).", self.scan_rate_hz),
                        Color32::from_rgb(0, 229, 255),
                    ));
                }
                Err(e) => {
                    self.terminal_history.push((
                        format!("[ERROR] Failed to start scan: {}", e),
                        Color32::from_rgb(255, 23, 68),
                    ));
                }
            }
        }
        self.finish_command_pending();
    }

    pub fn handle_stop_scan(&mut self) {
        if let Some(c) = self.client.as_mut() {
            c.stop_scan();
            self.is_scanning = false;
            self.terminal_history.push((
                "■ SSEP Scan stopped. Sockets returned to idle.".into(),
                Color32::from_rgb(255, 145, 0),
            ));
        }
    }

    pub fn handle_calz_target(&mut self, target: Option<usize>) {
        if !self.is_connected || self.is_calibrating {
            return;
        }

        let target_str = match target {
            None => "전체 동시 (All)".to_string(),
            Some(i) => format!("모듈 {}", i),
        };

        // Temporarily pause scan if running so command socket has full exclusive bandwidth
        self.calz_was_scanning = self.is_scanning;
        if self.is_scanning {
            self.handle_stop_scan();
        }

        // Send hardware CALZ command directly over the active command stream
        if let Some(c) = self.client.as_mut() {
            let _ = c.send_command_to(target, "CALZ");
        }

        self.is_calibrating = true;
        self.calz_start_time = Some(Instant::now());
        self.calz_target = target;
        self.calz_progress_pct = 10;
        self.calz_status_text = format!("하드웨어 CALZ 명령 전송 완료 ({}). 공압 밸브 CAL 전환 및 안정화 대기 중...", target_str);
        self.terminal_history.push((
            format!("⚡ [CALZ] 영점 교정 시작 ({}) - 밸브가 CAL 모드로 자동 전환되어 영점 안정화(3.5초)를 진행합니다.", target_str),
            Color32::from_rgb(0, 229, 255),
        ));
    }

    pub fn handle_calz(&mut self) {
        self.handle_calz_target(self.control_target);
    }

    pub fn handle_zero_tare(&mut self, target: Option<usize>) {
        self.zero_software_offsets(target);
        let target_str = match target {
            None => "전체 채널".to_string(),
            Some(i) => format!("모듈 {}", i),
        };
        self.terminal_history.push((
            format!("⚡ [Zero Tare] {} 현재 측정 압력을 기준으로 모든 채널을 즉시 0.000 PSI로 보정하였습니다.", target_str),
            Color32::from_rgb(0, 230, 118),
        ));
    }

    pub fn handle_reset_offsets(&mut self, target: Option<usize>) {
        let (start_ch, end_ch) = match target {
            None => (0, self.channel_offsets.len()),
            Some(idx) => (
                idx * CHANNELS_PER_MODULE,
                ((idx + 1) * CHANNELS_PER_MODULE).min(self.channel_offsets.len()),
            ),
        };
        for ch in start_ch..end_ch {
            if ch < self.channel_offsets.len() {
                self.channel_offsets[ch] = 0.0;
            }
            if ch < self.channel_gains.len() {
                self.channel_gains[ch] = 1.0;
            }
        }
        self.data_history.clear();
        self.save_config();
        self.terminal_history.push((
            "⚡ [Zero Reset] 영점 오프셋 및 Gain 보정이 원본(0.000 / 1.0)으로 초기화되었습니다.".into(),
            Color32::from_rgb(255, 145, 0),
        ));
    }

    pub fn zero_software_offsets(&mut self, target: Option<usize>) {
        let (start_ch, end_ch) = match target {
            None => (0, self.channel_offsets.len()),
            Some(idx) => (
                idx * CHANNELS_PER_MODULE,
                ((idx + 1) * CHANNELS_PER_MODULE).min(self.channel_offsets.len()),
            ),
        };

        for ch in start_ch..end_ch {
            let cur_raw = if let Some(st) = self.channel_stats.get(ch) {
                if st.raw_current != 0.0 {
                    st.raw_current
                } else if st.current != 0.0 {
                    st.current
                } else {
                    0.0
                }
            } else {
                0.0
            };

            let gain = self.channel_gains.get(ch).copied().unwrap_or(1.0);
            if ch < self.channel_offsets.len() {
                self.channel_offsets[ch] = -(cur_raw * gain);
            }

            if let Some(st) = self.channel_stats.get_mut(ch) {
                st.current = 0.0;
                st.calc_current = 0.0;
                st.min = 0.0;
                st.max = 0.0;
                st.sum = 0.0;
                st.sum_sq = 0.0;
                st.count = 1;
            }
        }

        self.data_history.clear();
        self.save_config();
    }

    pub fn zero_single_channel_offset(&mut self, ch: usize) {
        if ch >= self.channel_offsets.len() {
            return;
        }
        let cur_raw = if let Some(st) = self.channel_stats.get(ch) {
            if st.raw_current != 0.0 {
                st.raw_current
            } else if st.current != 0.0 {
                st.current
            } else {
                0.0
            }
        } else {
            0.0
        };

        let gain = self.channel_gains.get(ch).copied().unwrap_or(1.0);
        self.channel_offsets[ch] = -(cur_raw * gain);

        if let Some(st) = self.channel_stats.get_mut(ch) {
            st.current = 0.0;
            st.calc_current = 0.0;
            st.min = 0.0;
            st.max = 0.0;
            st.sum = 0.0;
            st.sum_sq = 0.0;
            st.count = 1;
        }
        self.save_config();
    }

    fn tick_calibration_state(&mut self) {
        if !self.is_calibrating {
            return;
        }

        let elapsed = self.calz_start_time.as_ref().map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
        if elapsed < 3.2 {
            self.calz_progress_pct = (10.0 + (elapsed / 3.2) * 65.0).clamp(10.0, 75.0) as u32;
            self.calz_status_text = format!("공압 솔레노이드 CAL 전환 및 영점 안정화 중... ({:.1}s / 3.5s)", elapsed);
        } else if elapsed < 4.2 {
            self.calz_progress_pct = (75.0 + ((elapsed - 3.2) / 1.0) * 20.0).clamp(75.0, 95.0) as u32;
            self.calz_status_text = "하드웨어 영점 Flash EEPROM 저장 (SAVE) 및 PX 복귀 중...".into();
        } else {
            // Complete calibration!
            if let Some(c) = self.client.as_mut() {
                let _ = c.send_command_to(self.calz_target, "SAVE");
                let _ = c.set_valve_target(self.calz_target, ValveMode::Px);
            }
            if let Some(target) = self.calz_target {
                if target < self.module_valves.len() {
                    self.module_valves[target] = "PX".into();
                }
            } else {
                for v in &mut self.module_valves {
                    *v = "PX".into();
                }
                self.system_info.valve_mode = "PX".into();
            }

            // Sync software zero tare offsets so that screen immediately shows 0.000 PSI!
            self.zero_software_offsets(self.calz_target);

            self.is_calibrating = false;
            self.calz_progress_pct = 100;
            self.calz_status_text = "영점 교정(CALZ) 및 0.000 PSI 동기화 완료!".into();
            self.calz_start_time = None;

            self.terminal_history.push((
                "⚡ [CALZ] 영점 교정 완료! 밸브 PX 복귀, EEPROM 영점 저장(SAVE) 및 채널 0.000 PSI 보정이 적용되었습니다.".into(),
                Color32::from_rgb(0, 230, 118),
            ));

            if self.calz_was_scanning {
                self.handle_start_scan();
                self.calz_was_scanning = false;
            }
        }
        self.finish_command_pending();
    }

    fn render_calz_modal(&mut self, ctx: &egui::Context) {
        egui::Window::new("⚡ DSA5000 하드웨어 영점 교정 (CALZ)")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(Color32::from_rgb(18, 22, 30))
                    .stroke(Stroke::new(1.5_f32, Color32::from_rgb(0, 229, 255)))
                    .inner_margin(egui::Margin::same(16.0))
            )
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("하드웨어 영점 교정 진행 중").heading().color(Color32::WHITE));
                    ui.add_space(8.0);
                    ui.label(RichText::new(&self.calz_status_text).strong().size(13.5).color(Color32::from_rgb(0, 229, 255)));
                    ui.add_space(12.0);
                    let progress = (self.calz_progress_pct as f32 / 100.0).clamp(0.0, 1.0);
                    ui.add(egui::ProgressBar::new(progress).show_percentage().animate(true));
                    ui.add_space(10.0);
                    ui.label(RichText::new("내부 공압 밸브가 CAL 모드로 자동 전환되어 센서 영점을 측정 중입니다.\n교정 완료 후 Flash EEPROM에 영구 저장(SAVE)되고 PX(측정) 모드로 자동 복귀합니다.")
                        .size(11.5)
                        .color(Color32::GRAY));
                    ui.add_space(6.0);
                });
            });
    }

    pub fn handle_set_valve_target(&mut self, target: Option<usize>, mode: ValveMode) {
        let mode_str = mode.as_str().to_string();
        self.trigger_command_pending(format!("공압 밸브 {} 전환 중...", mode.as_str()));
        if let Some(c) = self.client.as_mut() {
            match c.set_valve_target(target, mode) {
                Ok(resp) => {
                    match target {
                        None => {
                            self.system_info.valve_mode = mode_str.clone();
                            for v in &mut self.module_valves {
                                *v = mode_str.clone();
                            }
                        }
                        Some(idx) => {
                            if idx < self.module_valves.len() {
                                self.module_valves[idx] = mode_str.clone();
                            }
                            if idx == 0 {
                                self.system_info.valve_mode = mode_str.clone();
                            }
                        }
                    }
                    self.terminal_history.push((
                        format!("[VALVE] {}", resp),
                        Color32::from_rgb(0, 230, 118),
                    ));
                }
                Err(e) => {
                    self.terminal_history.push((
                        format!("[ERROR] Valve change failed: {}", e),
                        Color32::from_rgb(255, 23, 68),
                    ));
                }
            }
        }
        self.finish_command_pending();
    }

    pub fn handle_set_valve(&mut self, mode: ValveMode) {
        self.handle_set_valve_target(self.control_target, mode);
    }

    pub fn handle_set_ssep_assignment(&mut self, target: Option<usize>, assignment: u8) {
        let role = crate::protocol::SsepAssignment::from_u8(assignment);
        self.ssep_assignment = role;

        if let Some(idx) = target {
            if let Some(dev) = self.standalone_devices.get_mut(idx) {
                dev.ssep_role = role;
            }
        }

        if let Some(c) = self.client.as_mut() {
            match c.set_ssep_mode(target, assignment) {
                Ok(resp) => {
                    let target_desc = match target {
                        None => "전체 모듈".to_string(),
                        Some(i) => format!("모듈 {}", i),
                    };
                    self.terminal_history.push((
                        format!("[SSEP ROLE] SSEP {} ({}) {} 적용 완료: {}", assignment, role.label(), target_desc, resp.trim()),
                        Color32::from_rgb(0, 229, 255),
                    ));
                    if self.operating_mode == OperatingMode::Ssep {
                        if let Ok(st) = c.query_ssep_system() {
                            self.ssep_state = st;
                        }
                    }
                }
                Err(e) => {
                    self.terminal_history.push((
                        format!("[ERROR] SSEP {} 설정 실패: {}", assignment, e),
                        Color32::from_rgb(255, 60, 60),
                    ));
                }
            }
        } else {
            self.terminal_history.push((
                format!("[SSEP ROLE] 로컬 설정 변경: SSEP {} ({})", assignment, role.label()),
                Color32::from_rgb(0, 229, 255),
            ));
        }

        self.save_config();
    }

    pub fn handle_save_hardware(&mut self, target: Option<usize>) {
        self.trigger_command_pending("하드웨어 영구 저장 (SAVE) 중...");
        if let Some(c) = self.client.as_mut() {
            match c.save_hardware(target) {
                Ok(resp) => {
                    let target_desc = match target {
                        None => "전체 장비".to_string(),
                        Some(i) => format!("모듈 {}", i),
                    };
                    self.terminal_history.push((
                        format!("💾 [SAVE] {} Flash EEPROM 영구 저장 완료: {}", target_desc, resp.trim()),
                        Color32::from_rgb(0, 230, 118),
                    ));
                }
                Err(e) => {
                    self.terminal_history.push((
                        format!("[ERROR] 하드웨어 Flash SAVE 실패: {}", e),
                        Color32::from_rgb(255, 60, 60),
                    ));
                }
            }
        }
        self.finish_command_pending();
    }

    pub fn handle_query_ssep(&mut self, target: Option<usize>) {
        if let Some(c) = self.client.as_mut() {
            match c.get_ssep_mode(target) {
                Ok(resp) => {
                    let target_desc = match target {
                        None => "전체 장비".to_string(),
                        Some(i) => format!("모듈 {}", i),
                    };
                    self.terminal_history.push((
                        format!("🔍 [SSEP QUERY] {} 현재 SSEP 모드: {}", target_desc, resp.trim()),
                        Color32::from_rgb(255, 234, 0),
                    ));
                }
                Err(e) => {
                    self.terminal_history.push((
                        format!("[ERROR] SSEP 조회 실패: {}", e),
                        Color32::from_rgb(255, 60, 60),
                    ));
                }
            }
        }
        self.finish_command_pending();
    }

    pub fn handle_apply_and_save_ssep_role(&mut self, target: usize, role: crate::protocol::SsepAssignment) {
        if let Some(ed) = self.standalone_devices.get_mut(target) {
            ed.ssep_role = role;
        }

        if let Some(c) = self.client.as_mut() {
            let role_val = role.to_u8();
            let host_label = self.standalone_devices.get(target).map(|d| d.host.as_str()).unwrap_or("장치");
            match c.set_ssep_mode(Some(target), role_val) {
                Ok(resp) => {
                    let save_res = c.save_hardware(Some(target));
                    self.terminal_history.push((
                        format!("✔ [SSEP & SAVE] 모듈 {} ({}) -> {} 적용 및 EEPROM 저장 완료 (응답: {}, SAVE: {:?})", target, host_label, role.label(), resp.trim(), save_res),
                        Color32::from_rgb(0, 230, 118),
                    ));
                    if self.operating_mode == OperatingMode::Ssep {
                        if let Ok(st) = c.query_ssep_system() {
                            self.ssep_state = st;
                        }
                    }
                }
                Err(e) => {
                    self.terminal_history.push((
                        format!("[ERROR] 모듈 {} SSEP 설정 실패: {}", target, e),
                        Color32::from_rgb(255, 60, 60),
                    ));
                }
            }
        } else {
            self.terminal_history.push((
                format!("💾 [SSEP ROLE] 로컬 설정 저장 완료: 모듈 {} -> {} (장비 연결 후 [적용 & SAVE] 버튼으로 하드웨어 전송 가능)", target, role.label()),
                Color32::from_rgb(0, 229, 255),
            ));
        }

        self.save_config();
    }

    pub fn handle_apply_designated_ssep_roles(&mut self) {
        if let Some(c) = self.client.as_mut() {
            let total = c.devices.len();
            if total == 0 {
                return;
            }
            self.terminal_history.push((
                format!("🚀 각 장치별 지정된 SSEP 역할 하드웨어 전송 및 Flash SAVE 시작 (총 {}대)...", total),
                Color32::from_rgb(255, 234, 0),
            ));
            for i in 0..total {
                let role = if let Some(ed) = self.standalone_devices.get(i) {
                    ed.ssep_role
                } else if i == 0 {
                    crate::protocol::SsepAssignment::Controller
                } else {
                    crate::protocol::SsepAssignment::MiddleResponder
                };
                let role_code = role.to_u8();
                let res = c.set_ssep_mode(Some(i), role_code);
                let save_res = c.save_hardware(Some(i));
                let dev_label = c.devices.get(i).map(|d| d.endpoint.host.as_str()).unwrap_or("Unknown");
                self.terminal_history.push((
                    format!("  -> 모듈 {} ({}): {} 적용 [{:?}] / Flash SAVE [{:?}]", i, dev_label, role.label(), res, save_res),
                    Color32::from_rgb(0, 229, 255),
                ));
            }
            self.terminal_history.push((
                "✔ 지정된 모든 SSEP 역할 하드웨어 전송 및 EEPROM 영구 저장(SAVE) 완료!".into(),
                Color32::from_rgb(0, 230, 118),
            ));
            if let Ok(st) = c.query_ssep_system() {
                self.ssep_state = st;
            }
            self.save_config();
        } else {
            self.terminal_history.push((
                "장비에 연결되어 있지 않습니다. 장비 연결 후 [적용 & SAVE]를 수행하세요.".into(),
                Color32::GRAY,
            ));
        }
    }

    pub fn handle_configure_ssep_chain(&mut self) {
        if let Some(c) = self.client.as_mut() {
            let total = c.devices.len();
            if total == 0 {
                return;
            }
            self.terminal_history.push((
                format!("🚀 SSEP 다중 모듈 체인 자동 구성 시작 (총 {}대)...", total),
                Color32::from_rgb(255, 234, 0),
            ));
            for i in 0..total {
                let (role_code, role_name) = if i == 0 {
                    (2u8, "Controller (SSEP 2)")
                } else if i == total - 1 {
                    (4u8, "Last Responder (SSEP 4)")
                } else {
                    (3u8, "Middle Responder (SSEP 3)")
                };

                let res = c.set_ssep_mode(Some(i), role_code);
                let save_res = c.save_hardware(Some(i));
                let dev_label = c.devices.get(i).map(|d| d.endpoint.host.as_str()).unwrap_or("Unknown");
                self.terminal_history.push((
                    format!("  -> 모듈 {} ({}): {} 적용 [{:?}] / Flash SAVE [{:?}]", i, dev_label, role_name, res, save_res),
                    Color32::from_rgb(0, 229, 255),
                ));

                if let Some(ed) = self.standalone_devices.get_mut(i) {
                    ed.ssep_role = crate::protocol::SsepAssignment::from_u8(role_code);
                }
            }
            self.terminal_history.push((
                "✔ SSEP 체인 하드웨어 구성 및 EEPROM 저장 완료!".into(),
                Color32::from_rgb(0, 230, 118),
            ));
            if let Ok(st) = c.query_ssep_system() {
                self.ssep_state = st;
            }
            self.save_config();
        }
    }

    pub fn handle_apply_scan_params(&mut self) {
        let rate = self.scan_rate_hz.clamp(1, 5000);
        let units = self.engineering_units.clone();
        let trig = self.trigger_mode;
        let fps = self.fps_average;
        let target = self.scan_params_target;

        if let Some(c) = self.client.as_mut() {
            let _ = c.set_rate_target(target, rate);
            let _ = c.set_units_target(target, &units);
            let _ = c.set_trigger_mode_target(target, trig);
            let _ = c.set_fps_target(target, fps);
            let target_str = match target {
                None => "전체 장비".into(),
                Some(i) => format!("모듈 {}", i),
            };
            self.terminal_history.push((
                format!("[*] Scan parameters applied to {}: Rate={} Hz (Max 5000), Units={}, Trig={}, FPS={}", target_str, rate, units, trig, fps),
                Color32::from_rgb(0, 229, 255),
            ));
        }

        self.system_info.scan_rate_hz = rate;
        self.system_info.engineering_units = units;

        self.save_config();
        self.show_scan_params_modal = false;
    }

    pub fn handle_send_terminal(&mut self) {
        let cmd = self.terminal_input.trim().to_string();
        if cmd.is_empty() {
            return;
        }

        self.trigger_command_pending(format!("명령 전송 중: {}", cmd));

        let target = self.terminal_target;
        let target_str = match target {
            None => "전체 동시 (Broadcast)".into(),
            Some(i) => format!("모듈 {}", i),
        };
        self.terminal_history.push((format!("> [{}] {}", target_str, cmd), Color32::WHITE));
        self.terminal_input.clear();

        if let Some(c) = self.client.as_mut() {
            match c.send_command_to(target, &cmd) {
                Ok(resp) => {
                    for line in resp.lines() {
                        self.terminal_history.push((line.to_string(), Color32::from_rgb(170, 255, 170)));
                    }
                }
                Err(e) => {
                    self.terminal_history.push((format!("ERROR: {}", e), Color32::from_rgb(255, 60, 60)));
                }
            }
        } else {
            self.terminal_history.push(("Not connected to any DSA5000 module.".into(), Color32::GRAY));
        }
        self.finish_command_pending();
    }

    pub fn toggle_logging(&mut self) {
        if self.logger.is_recording {
            if let Ok(Some(path)) = self.logger.stop() {
                let p_str = path.to_string_lossy().to_string();
                self.terminal_history.push((
                    format!("■ CSV Log saved: {} ({} frames)", p_str, self.logger.recorded_frames),
                    Color32::from_rgb(255, 234, 0),
                ));
                self.last_logged_path = Some(p_str);
            }
        } else {
            // Automatically start acquisition if not already scanning so data frames arrive
            if !self.is_scanning {
                self.handle_start_scan();
            }

            self.logger.single_file_mode = self.single_file_mode;
            self.logger.append_to_existing = self.append_to_existing;
            self.logger.set_channel_names(self.channel_names.clone());
            match self.logger.start(&self.system_info, &self.log_base_name) {
                Ok(path) => {
                    let p_str = path.to_string_lossy().to_string();
                    self.terminal_history.push((
                        format!("● CSV Logging started (Single File): {}", p_str),
                        Color32::from_rgb(255, 23, 68),
                    ));
                    self.last_logged_path = Some(p_str);
                }
                Err(e) => {
                    self.terminal_history.push((
                        format!("[ERROR] Failed to start log: {}", e),
                        Color32::from_rgb(255, 23, 68),
                    ));
                }
            }
        }
        self.finish_command_pending();
    }

    fn drain_incoming_packets(&mut self) {
        let mut packets_read = 0;

        while let Ok(pkt) = self.frame_rx.try_recv() {
            packets_read += 1;

            // Update latest IEEE-1588 PTP timing data
            self.last_ptp_sec = pkt.header.ptp_sec;
            self.last_ptp_nsec = pkt.header.ptp_nsec;

            // Log to CSV if active
            if self.logger.is_recording {
                let _ = self.logger.write_frame(&pkt);
            }

            // Calculate precise per-packet timestamp (PTP nanoseconds)
            let pkt_time = if pkt.header.ptp_sec > 0 {
                let ptp_t = (pkt.header.ptp_sec as f64) + (pkt.header.ptp_nsec as f64 / 1_000_000_000.0);
                if self.first_ptp_time.is_none() {
                    self.first_ptp_time = Some(ptp_t);
                }
                ptp_t - self.first_ptp_time.unwrap()
            } else {
                let elapsed = self.start_instant.elapsed().as_secs_f64();
                let dt = 1.0 / (self.scan_rate_hz as f64).max(1.0);
                let last = self.data_history.last().map(|(t, _)| *t).unwrap_or(elapsed);
                (last + dt).max(elapsed)
            };

            // Enforce strict monotonicity so no two packets share the same timestamp (prevents batch vertical line spikes)
            let final_time = if let Some(&(last_t, _)) = self.data_history.last() {
                if pkt_time <= last_t {
                    last_t + 0.0001
                } else {
                    pkt_time
                }
            } else {
                pkt_time
            };

            // Flatten pressures across all SSEP modules in packet
            let total_channels = pkt.total_channels();
            let mut all_pressures = Vec::with_capacity(total_channels);

            for (m_idx, m) in pkt.modules.iter().enumerate() {
                for (ch_idx, &p) in m.pressures.iter().enumerate() {
                    let global_ch = (m_idx * CHANNELS_PER_MODULE) + ch_idx;
                    let gain = self.channel_gains.get(global_ch).copied().unwrap_or(1.0);
                    let offset = self.channel_offsets.get(global_ch).copied().unwrap_or(0.0);
                    let calc_p = p * gain + offset;

                    all_pressures.push(calc_p);

                    // Update running statistics
                    if global_ch < self.channel_stats.len() {
                        let st = &mut self.channel_stats[global_ch];
                        st.raw_current = p;
                        st.current = calc_p;
                        st.calc_current = calc_p;
                        st.sum += calc_p as f64;
                        st.sum_sq += (calc_p as f64) * (calc_p as f64);
                        st.count += 1;
                        if calc_p < st.min {
                            st.min = calc_p;
                        }
                        if calc_p > st.max {
                            st.max = calc_p;
                        }
                        st.temp = m.temperatures[ch_idx];
                    }
                }
            }

            if !self.is_paused {
                self.data_history.push((final_time, all_pressures));
            }

            // Yield after high-volume batch
            if packets_read >= 250 {
                break;
            }
        }

        // Prune data history older than (time_window_sec + 15.0, min 75.0)
        // This ensures the 60.0s time window is never truncated!
        if !self.is_paused && !self.data_history.is_empty() {
            let latest_t = self.data_history.last().map(|(t, _)| *t).unwrap_or(0.0);
            let retain_sec = (self.time_window_sec + 15.0).max(75.0);
            let cutoff = latest_t - retain_sec;
            self.data_history.retain(|(t, _)| *t >= cutoff);
        }

        // Update stats from client
        if let Some(c) = self.client.as_ref() {
            self.client_stats = c.get_stats();
        }
    }

    /// Formats the latest PTP timestamp and inter-frame jitter for clean, uniform display
    pub fn format_ptp_display(&self) -> (String, String) {
        let sec = if self.last_ptp_sec > 0 {
            self.last_ptp_sec
        } else {
            self.client_stats.last_ptp_sec
        };
        let nsec = if self.last_ptp_nsec > 0 {
            self.last_ptp_nsec
        } else {
            self.client_stats.last_ptp_nsec
        };
        let jitter = self.client_stats.ptp_jitter_ns;

        if sec == 0 && nsec == 0 {
            ("PTP: 0.000000000s".to_string(), "지터: 0 ns".to_string())
        } else {
            let time_str = if sec > 1_600_000_000 {
                // Realistic Epoch timestamp (convert to local HH:MM:SS.mmm)
                let s_in_day = sec % 86400;
                let h = (s_in_day / 3600 + 9) % 24; // KST (+9h)
                let m = (s_in_day % 3600) / 60;
                let s = s_in_day % 60;
                let ms = nsec / 1_000_000;
                format!("{:02}:{:02}:{:02}.{:03} ({}.{:09}s)", h, m, s, ms, sec, nsec)
            } else {
                format!("{}.{:09}s", sec, nsec)
            };
            let jitter_str = format!("지터: {:+} ns", jitter);
            (format!("PTP: {}", time_str), jitter_str)
        }
    }
}

impl eframe::App for DsaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_incoming_packets();

        // Manage command waiting state & minimum visual feedback timeout
        if self.is_command_pending {
            if let Some(start) = self.command_pending_start {
                if start.elapsed() >= Duration::from_millis(600) {
                    self.is_command_pending = false;
                    self.command_pending_start = None;
                    if !self.is_calibrating {
                        set_win32_busy_cursor(false);
                    }
                }
            }
        }

        // Top Header Bar
        egui::TopBottomPanel::top("top_header").show(ctx, |ui| {
            self.render_header_bar(ui);
        });

        // Ribbon Action Bar
        egui::TopBottomPanel::top("ribbon_panel").show(ctx, |ui| {
            self.render_ribbon(ui);
        });

        // Scan Parameters Modal Dialog
        if self.show_scan_params_modal {
            self.render_scan_params_window(ctx);
        }

        // Channel Names Modal Dialog
        if self.show_channel_names_modal {
            self.render_channel_names_window(ctx);
        }

        // Multi-Device Manager Modal Dialog
        if self.show_device_manager_modal {
            self.render_device_manager_window(ctx);
        }

        // Calibration Progress Modal Dialog
        if self.is_calibrating {
            self.tick_calibration_state();
            self.render_calz_modal(ctx);
        }

        // Bottom Status Bar
        egui::TopBottomPanel::bottom("bottom_status").show(ctx, |ui| {
            self.render_bottom_status_bar(ui);
        });

        // Central Content View
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected_tab, AppTab::Oscilloscope, "📊 실시간 오실로스코프");
                ui.selectable_value(&mut self.selected_tab, AppTab::BarMeters, "📶 채널별 바미터 (바둑판)");
                ui.selectable_value(&mut self.selected_tab, AppTab::Statistics, "📈 실시간 통계 분석");
                ui.selectable_value(&mut self.selected_tab, AppTab::SsepTopology, "🔗 SSEP 토폴로지/장비");
                ui.selectable_value(&mut self.selected_tab, AppTab::DataLogger, "💾 CSV 데이터 로깅");
                ui.selectable_value(&mut self.selected_tab, AppTab::Terminal, "💻 Telnet 터미널");
            });

            ui.separator();

            match self.selected_tab {
                AppTab::Oscilloscope => self.render_oscilloscope_tab(ui),
                AppTab::BarMeters => self.render_bar_meters_tab(ui),
                AppTab::Statistics => self.render_statistics_tab(ui),
                AppTab::SsepTopology => self.render_ssep_topology_tab(ui),
                AppTab::DataLogger => self.render_data_logger_tab(ui),
                AppTab::Terminal => self.render_terminal_tab(ui),
            }
        });

        // Continuous high-speed 60 FPS repainting when scanning, connected, calibrating, or command pending
        if self.is_scanning || self.is_connected || self.is_calibrating || self.is_command_pending {
            ctx.request_repaint();
        }

        // Set mouse cursor to Wait / Progress when command response is pending or calibrating
        if self.is_calibrating || self.is_command_pending {
            ctx.set_cursor_icon(egui::CursorIcon::Wait);
            set_win32_busy_cursor(true);
        }
    }
}

impl DsaApp {
    fn render_header_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("DSA5000 STUDIO").strong().color(Color32::from_rgb(0, 229, 255)));
            let mode_tag = if self.operating_mode == OperatingMode::Standalone {
                "STANDALONE v2.01"
            } else {
                "SSEP NATIVE v2.01"
            };
            ui.label(RichText::new(mode_tag).small().color(Color32::GRAY));

            ui.separator();

            // Status Badge
            let (badge_text, badge_color) = if !self.is_connected {
                ("DISCONNECTED", Color32::from_rgb(255, 60, 60))
            } else if self.is_scanning {
                if self.operating_mode == OperatingMode::Standalone {
                    ("ACQUIRING [STANDALONE 16Ch]", Color32::from_rgb(0, 229, 255))
                } else {
                    ("ACQUIRING DATA [SSEP]", Color32::from_rgb(0, 229, 255))
                }
            } else {
                if self.operating_mode == OperatingMode::Standalone {
                    ("CONNECTED [STANDALONE READY]", Color32::from_rgb(0, 230, 118))
                } else {
                    ("CONNECTED [SSEP READY]", Color32::from_rgb(0, 230, 118))
                }
            };
            ui.colored_label(badge_color, RichText::new(format!("● {}", badge_text)).strong());

            ui.separator();

            // Operating Mode Selector: SSEP vs Standalone
            ui.label(RichText::new("모드:").strong());
            if ui.selectable_label(self.operating_mode == OperatingMode::Ssep, "SSEP (다중 동기화)").clicked() {
                self.switch_operating_mode(OperatingMode::Ssep);
            }
            if ui.selectable_label(self.operating_mode == OperatingMode::Standalone, "스탠드얼론 (단독/다중)").clicked() {
                self.switch_operating_mode(OperatingMode::Standalone);
            }

            if self.operating_mode == OperatingMode::Ssep {
                ui.separator();
                let cur_label = self.ssep_assignment.label();
                ui.label(RichText::new("SSEP 모드:").strong());
                egui::ComboBox::from_id_salt("header_ssep_role_combo")
                    .selected_text(cur_label)
                    .show_ui(ui, |ui| {
                        for role in [
                            crate::protocol::SsepAssignment::Controller,
                            crate::protocol::SsepAssignment::MiddleResponder,
                            crate::protocol::SsepAssignment::LastResponder,
                            crate::protocol::SsepAssignment::Auto,
                            crate::protocol::SsepAssignment::Standalone,
                        ] {
                            if ui.selectable_value(&mut self.ssep_assignment, role, role.label()).clicked() {
                                self.save_config();
                                if self.is_connected {
                                    self.handle_set_ssep_assignment(None, role.to_u8());
                                }
                            }
                        }
                    });
            }

            ui.separator();

            // Connection Inputs
            ui.label("IP:");
            let ip_w = 160.0;
            if ui.add(egui::TextEdit::singleline(&mut self.host).desired_width(ip_w)).changed() {
                self.sync_devices_from_host_string();
                self.save_config();
            }

            let cnt = self.standalone_devices.len().max(1);
            let btn_text = format!("⚙ 장치 및 SSEP 설정 ({}대)", cnt);
            if ui.button(RichText::new(btn_text).color(Color32::from_rgb(0, 229, 255)).strong()).clicked() {
                self.show_device_manager_modal = true;
            }

            if self.operating_mode == OperatingMode::Ssep && self.is_connected && self.standalone_devices.len() > 1 {
                if ui.button(RichText::new("🚀 SSEP 일괄 적용 & SAVE").strong().color(Color32::from_rgb(0, 230, 118)))
                    .on_hover_text("지정된 Controller 및 Responder 역할을 각 장치에 적용하고 EEPROM에 영구 저장합니다.")
                    .clicked()
                {
                    self.handle_apply_designated_ssep_roles();
                }
            }

            ui.label("Cmd:");
            let mut cmd_str = self.cmd_port.to_string();
            if ui.add(egui::TextEdit::singleline(&mut cmd_str).desired_width(40.0)).changed() {
                if let Ok(p) = cmd_str.parse() {
                    self.cmd_port = p;
                    self.save_config();
                }
            }

            ui.label("Data:");
            let mut data_str = self.data_port.to_string();
            if ui.add(egui::TextEdit::singleline(&mut data_str).desired_width(40.0)).changed() {
                if let Ok(p) = data_str.parse() {
                    self.data_port = p;
                    self.save_config();
                }
            }

            // Simulator Mode Toggle
            if ui.checkbox(&mut self.is_simulator, "시뮬레이터 모드").changed() {
                if self.is_simulator {
                    self.host = "127.0.0.1".into();
                    self.cmd_port = 2323;
                    self.data_port = 5503;
                } else {
                    self.host = "192.168.1.100".into();
                    self.cmd_port = 23;
                    self.data_port = 503;
                }
                self.save_config();
            }

            // In simulator mode, show module/device count dropdown
            if self.is_simulator && !self.is_connected {
                let count_lbl = if self.operating_mode == OperatingMode::Standalone { "장치 수:" } else { "모듈 수:" };
                ui.label(count_lbl);
                let prev_count = self.sim_module_count;
                egui::ComboBox::from_id_salt("sim_mod_count")
                    .selected_text(format!("{}대 ({} Ch)", self.sim_module_count, self.sim_module_count * 16))
                    .show_ui(ui, |ui| {
                        let typ = if self.operating_mode == OperatingMode::Standalone { "독립형" } else { "SSEP" };
                        ui.selectable_value(&mut self.sim_module_count, 1, format!("1대 (16 Ch - {})", typ));
                        ui.selectable_value(&mut self.sim_module_count, 2, format!("2대 (32 Ch - {} 2대)", typ));
                        ui.selectable_value(&mut self.sim_module_count, 3, format!("3대 (48 Ch - {} 3대)", typ));
                        ui.selectable_value(&mut self.sim_module_count, 4, format!("4대 (64 Ch - {} 4대)", typ));
                        ui.selectable_value(&mut self.sim_module_count, 5, format!("5대 (80 Ch - {} 5대)", typ));
                        ui.selectable_value(&mut self.sim_module_count, 6, format!("6대 (96 Ch - {} 6대)", typ));
                        ui.selectable_value(&mut self.sim_module_count, 7, format!("7대 (112 Ch - {} 7대)", typ));
                        ui.selectable_value(&mut self.sim_module_count, 8, format!("8대 (128 Ch - {} 8대)", typ));
                    });
                if self.sim_module_count != prev_count {
                    if self.operating_mode == OperatingMode::Standalone {
                        self.standalone_devices.clear();
                        for i in 0..self.sim_module_count {
                            self.standalone_devices.push(
                                crate::client::DeviceEndpoint::new("127.0.0.1", self.cmd_port + i as u16, self.data_port + i as u16)
                                    .with_label(&format!("시뮬레이터 장치 {}", i + 1))
                            );
                        }
                        self.sync_host_string_from_devices();
                    }
                    self.save_config();
                }
            }

            // Connect / Disconnect Buttons
            if !self.is_connected {
                if ui.button(RichText::new("연결 (Connect)").strong().color(Color32::from_rgb(0, 230, 118))).clicked() {
                    self.handle_connect();
                }
            } else {
                if ui.button(RichText::new("연결 해제 (Disconnect)").color(Color32::from_rgb(255, 60, 60))).clicked() {
                    self.handle_disconnect();
                }
            }
        });
    }

    fn render_ribbon(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Acquisition Control
            ui.group(|ui| {
                ui.label(RichText::new("수집 제어").small().color(Color32::GRAY));
                if !self.is_scanning {
                    let btn = ui.add_enabled(
                        self.is_connected,
                        egui::Button::new(RichText::new("▶ 데이터 수집 시작 (SCAN)").strong().color(Color32::from_rgb(0, 230, 118))),
                    );
                    if btn.clicked() {
                        self.handle_start_scan();
                    }
                } else {
                    if ui.button(RichText::new("■ 데이터 수집 정지 (STOP)").strong().color(Color32::from_rgb(255, 60, 60))).clicked() {
                        self.handle_stop_scan();
                    }
                }

                let mod_cnt = self.current_module_count();
                let calz_label = if mod_cnt > 1 {
                    match self.control_target {
                        None => "⚡ 영점 교정 (CALZ 전체)".to_string(),
                        Some(idx) => format!("⚡ 모듈 {} 교정 (CALZ)", idx),
                    }
                } else {
                    "⚡ 영점 교정 (CALZ)".to_string()
                };
                if ui.add_enabled(self.is_connected && !self.is_calibrating, egui::Button::new(calz_label)).clicked() {
                    self.handle_calz();
                }
                if ui.add_enabled(self.is_connected && !self.is_calibrating, 
                    egui::Button::new(RichText::new("⚡ 즉각 영점 (Tare)").color(Color32::from_rgb(0, 230, 118)))
                ).on_hover_text("현재 측정 압력을 기준으로 즉시 모든 채널을 0.000 PSI로 설정합니다 (공압 밸브 전환 대기 없음)").clicked() {
                    self.handle_zero_tare(self.control_target);
                }
                if ui.add_enabled(self.is_connected && !self.is_calibrating, 
                    egui::Button::new(RichText::new("⚡ 영점 리셋").color(Color32::from_rgb(255, 145, 0)))
                ).on_hover_text("영점 오프셋 및 Gain 보정을 원본(0.000 / 1.0)으로 초기화합니다").clicked() {
                    self.handle_reset_offsets(self.control_target);
                }
            });

            // System Settings & Channel Names Buttons
            ui.group(|ui| {
                ui.label(RichText::new("시스템 설정 / 채널").small().color(Color32::GRAY));
                let btn = ui.button(
                    RichText::new(format!("⚙ 시스템 설정 ({} Hz / {} / 밸브: {})", self.scan_rate_hz, self.engineering_units, self.system_info.valve_mode))
                        .strong()
                        .color(Color32::from_rgb(255, 234, 0)),
                );
                if btn.clicked() {
                    self.show_scan_params_modal = !self.show_scan_params_modal;
                }

                if ui.button(
                    RichText::new("🏷 채널 이름 및 Gain/Offset 설정")
                        .strong()
                        .color(Color32::from_rgb(0, 229, 255)),
                ).clicked() {
                    self.show_channel_names_modal = !self.show_channel_names_modal;
                }
            });

            // SSEP / Hardware Discovery Refresh
            ui.group(|ui| {
                let disc_title = if self.operating_mode == OperatingMode::Standalone {
                    "장치 상태"
                } else {
                    "SSEP 라인"
                };
                let btn_lbl = if self.operating_mode == OperatingMode::Standalone {
                    "🔄 장치 상태 새로고침"
                } else {
                    "🔄 SSEP 모듈 검색"
                };
                ui.label(RichText::new(disc_title).small().color(Color32::GRAY));
                if ui.add_enabled(self.is_connected, egui::Button::new(btn_lbl)).clicked() {
                    if let Some(c) = self.client.as_mut() {
                        if let Ok(st) = c.query_ssep_system() {
                            self.ssep_state = st;
                        }
                        self.module_valves = c.get_device_valves();
                        let cur_cnt = self.current_module_count();
                        if self.module_valves.len() < cur_cnt {
                            self.module_valves.resize(cur_cnt, "PX".into());
                        }
                    }
                }
            });

            // Fast CSV Logging Quick Toggle
            ui.group(|ui| {
                ui.label(RichText::new("CSV 로깅").small().color(Color32::GRAY));
                let (btn_txt, btn_color) = if self.logger.is_recording {
                    (format!("■ REC 정지 ({})", self.logger.recorded_frames), Color32::from_rgb(255, 60, 60))
                } else {
                    ("● REC 시작".into(), Color32::from_rgb(0, 230, 118))
                };
                if ui.add_enabled(self.is_connected, egui::Button::new(RichText::new(btn_txt).color(btn_color))).clicked() {
                    self.toggle_logging();
                }
            });
        });
    }

    /// System & Scan Parameters Configuration Dialog (Settings persistence, Max 5,000 Hz spec, Pneumatic Valve Control)
    fn render_scan_params_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("⚙ DSA5000 시스템 및 스캔 파라미터 설정 (System & Scan Parameters)")
            .collapsible(false)
            .resizable(false)
            .default_width(500.0)
            .show(ctx, |ui| {
                ui.heading(RichText::new("DSA5000 시스템 파라미터 및 공압밸브 제어").color(Color32::from_rgb(0, 229, 255)));
                ui.label("Scanivalve 공식 매뉴얼 규격: 최대 스캔 속도 5,000 Hz, 공압 솔레노이드 밸브 4대 모드(Px/Cal/Prg/Iso) 제어");
                ui.separator();

                let mod_cnt = self.current_module_count();
                if mod_cnt > 1 {
                    ui.group(|ui| {
                        ui.label(RichText::new("파라미터 / 밸브 제어 대상:").strong().color(Color32::from_rgb(0, 229, 255)));
                        ui.horizontal_wrapped(|ui| {
                            if ui.selectable_label(self.scan_params_target.is_none(), "🌐 전체 장치 동시 적용 (All)").clicked() {
                                self.scan_params_target = None;
                                self.control_target = None;
                            }
                            for i in 0..mod_cnt {
                                let label = format!("모듈 {} (Ch {:02}-{})", i, i * 16 + 1, (i + 1) * 16);
                                if ui.selectable_label(self.scan_params_target == Some(i), label).clicked() {
                                    self.scan_params_target = Some(i);
                                    self.control_target = Some(i);
                                }
                            }
                        });
                    });
                }

                // 1. Pneumatic Solenoid Valve Control
                ui.group(|ui| {
                    ui.label(RichText::new("공압 솔레노이드 밸브 모드 제어 (Pneumatic Valve):").strong().color(Color32::from_rgb(0, 229, 255)));
                    let curr_v = match self.scan_params_target {
                        None => self.system_info.valve_mode.clone(),
                        Some(idx) => self.module_valves.get(idx).cloned().unwrap_or_else(|| self.system_info.valve_mode.clone()),
                    };

                    ui.horizontal(|ui| {
                        let mut new_valve = None;
                        if ui.selectable_label(curr_v == "PX", "Px (계측 RUN)").clicked() {
                            new_valve = Some(ValveMode::Px);
                        }
                        if ui.selectable_label(curr_v == "CAL", "Cal (교정 CAL)").clicked() {
                            new_valve = Some(ValveMode::Cal);
                        }
                        if ui.selectable_label(curr_v == "PRG", "Prg (퍼지 PURGE)").clicked() {
                            new_valve = Some(ValveMode::Prg);
                        }
                        if ui.selectable_label(curr_v == "ISO", "Iso (차단 ISO)").clicked() {
                            new_valve = Some(ValveMode::Iso);
                        }
                        if let Some(v) = new_valve {
                            self.handle_set_valve_target(self.scan_params_target, v);
                        }

                        ui.separator();
                        ui.label("현재 밸브:");
                        ui.colored_label(Color32::from_rgb(0, 230, 118), RichText::new(&curr_v).strong());
                    });
                });

                // 1. Scan Rate (Hz)
                ui.group(|ui| {
                    ui.label(RichText::new("스캔 속도 (Scan Rate, Hz):").strong());
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.scan_rate_hz == 100, "100 Hz").clicked() {
                            self.scan_rate_hz = 100;
                        }
                        if ui.selectable_label(self.scan_rate_hz == 500, "500 Hz").clicked() {
                            self.scan_rate_hz = 500;
                        }
                        if ui.selectable_label(self.scan_rate_hz == 1000, "1,000 Hz").clicked() {
                            self.scan_rate_hz = 1000;
                        }
                        if ui.selectable_label(self.scan_rate_hz == 2500, "2,500 Hz").clicked() {
                            self.scan_rate_hz = 2500;
                        }
                        if ui.selectable_label(self.scan_rate_hz == 5000, "5,000 Hz (최대)").clicked() {
                            self.scan_rate_hz = 5000;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("직접 입력 (1 ~ 5000 Hz):");
                        ui.add(egui::DragValue::new(&mut self.scan_rate_hz).range(1..=5000).suffix(" Hz"));
                    });
                });

                // 2. Engineering Units
                ui.group(|ui| {
                    ui.label(RichText::new("공학 압력 단위 (Engineering Units):").strong());
                    ui.horizontal_wrapped(|ui| {
                        let units = ["PSI", "KPA", "PA", "BAR", "MBAR", "ATM", "MMHG", "INHG", "INH2O"];
                        for u in units {
                            if ui.selectable_label(self.engineering_units == u, u).clicked() {
                                self.engineering_units = u.to_string();
                            }
                        }
                    });
                });

                // 3. Trigger Mode
                ui.group(|ui| {
                    ui.label(RichText::new("트리거 모드 (Trigger Mode):").strong());
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.trigger_mode, 0, "0: Free Run (연속 자동 스캔)");
                        ui.selectable_value(&mut self.trigger_mode, 1, "1: External (외부 하드웨어 트리거)");
                        ui.selectable_value(&mut self.trigger_mode, 2, "2: Software (소프트웨어 트리거)");
                    });
                });

                // 4. FPS Decimation / Average
                ui.group(|ui| {
                    ui.label(RichText::new("FPS 전송 필터 / 데시메이션 (Decimation):").strong());
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.fps_average, 0, "0: 전체 전송 (No Decimation)");
                        ui.selectable_value(&mut self.fps_average, 10, "10 FPS");
                        ui.selectable_value(&mut self.fps_average, 50, "50 FPS");
                        ui.selectable_value(&mut self.fps_average, 100, "100 FPS");
                    });
                });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button(RichText::new("✔ 하드웨어 적용 및 저장 (Apply & Save)").strong().color(Color32::from_rgb(0, 230, 118))).clicked() {
                        self.handle_apply_scan_params();
                    }
                    if ui.button("닫기 (Close)").clicked() {
                        self.show_scan_params_modal = false;
                    }
                });
            });
    }

    fn render_channel_names_window(&mut self, ctx: &egui::Context) {
        let mut is_open = true;
        egui::Window::new("🏷 DSA5000 채널 이름 및 Gain / Offset 설정 (Channel & Calibration Config)")
            .open(&mut is_open)
            .collapsible(false)
            .resizable(true)
            .default_width(980.0)
            .default_height(680.0)
            .min_width(750.0)
            .min_height(350.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.heading(RichText::new("채널 식별명 및 개별 Gain / Offset 보정 계수 설정").color(Color32::from_rgb(0, 229, 255)));
                ui.label("각 채널에 식별 이름과 환산 보정식 (계산값 = 원본 측정값 × Gain + Offset)을 지정합니다. 창 우측 하단 또는 상하 테두리를 드래그하여 높이를 자유롭게 조절할 수 있습니다.");
                ui.separator();

                // 1. Quick Batch Tools (Prefix & Gain/Offset)
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("⚡ 이름 접두사 일괄 부여:").strong());
                        ui.add(egui::TextEdit::singleline(&mut self.channel_name_prefix).desired_width(100.0).hint_text("예: Wing_"));

                        let target_mod = self.channel_name_selected_mod;
                        let mod_lbl = if target_mod == 99 {
                            "전체 채널 (128 Ch)".to_string()
                        } else {
                            format!("모듈 {} (16 Ch)", target_mod)
                        };

                        if ui.button(RichText::new(format!("⚡ {} 이름 적용", mod_lbl)).strong().color(Color32::from_rgb(0, 230, 118))).clicked() {
                            let prefix = self.channel_name_prefix.trim().to_string();
                            if !prefix.is_empty() {
                                if target_mod == 99 {
                                    for i in 0..128 {
                                        if i < self.channel_names.len() {
                                            self.channel_names[i] = format!("{}{:02}", prefix, i + 1);
                                        }
                                    }
                                } else {
                                    for ch_local in 0..16 {
                                        let global_ch = target_mod * 16 + ch_local;
                                        if global_ch < self.channel_names.len() {
                                            self.channel_names[global_ch] = format!("{}{:02}", prefix, ch_local + 1);
                                        }
                                    }
                                }
                                self.save_config();
                            }
                        }

                        if ui.button("🗑 이름 비우기").clicked() {
                            if target_mod == 99 {
                                for i in 0..128 {
                                    if i < self.channel_names.len() {
                                        self.channel_names[i].clear();
                                    }
                                }
                            } else {
                                for ch_local in 0..16 {
                                    let global_ch = target_mod * 16 + ch_local;
                                    if global_ch < self.channel_names.len() {
                                        self.channel_names[global_ch].clear();
                                    }
                                }
                            }
                            self.save_config();
                        }
                    });

                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("⚡ Gain / Offset 일괄 설정:").strong().color(Color32::from_rgb(0, 229, 255)));
                        ui.label("Gain:");
                        ui.add(egui::DragValue::new(&mut self.batch_gain).speed(0.01).range(-100000.0..=100000.0).custom_formatter(|n, _| format!("{:.4}", n)));
                        ui.label("Offset:");
                        ui.add(egui::DragValue::new(&mut self.batch_offset).speed(0.01).range(-100000.0..=100000.0).custom_formatter(|n, _| format!("{:.4}", n)));

                        let target_mod = self.channel_name_selected_mod;
                        let mod_lbl = if target_mod == 99 {
                            "전체 채널".to_string()
                        } else {
                            format!("모듈 {}", target_mod)
                        };

                        if ui.button(RichText::new(format!("⚡ {}에 Gain/Offset 적용", mod_lbl)).strong().color(Color32::from_rgb(0, 229, 255))).clicked() {
                            let bg = self.batch_gain;
                            let bo = self.batch_offset;
                            if target_mod == 99 {
                                for i in 0..128 {
                                    if i < self.channel_gains.len() {
                                        self.channel_gains[i] = bg;
                                    }
                                    if i < self.channel_offsets.len() {
                                        self.channel_offsets[i] = bo;
                                    }
                                }
                            } else {
                                for ch_local in 0..16 {
                                    let global_ch = target_mod * 16 + ch_local;
                                    if global_ch < self.channel_gains.len() {
                                        self.channel_gains[global_ch] = bg;
                                    }
                                    if global_ch < self.channel_offsets.len() {
                                        self.channel_offsets[global_ch] = bo;
                                    }
                                }
                            }
                            self.save_config();
                        }

                        if ui.button(RichText::new("⚡ 현재 대상 채널 영점 (Tare)").color(Color32::from_rgb(0, 230, 118))).on_hover_text("현재 측정 압력을 기준으로 영점 오프셋을 자동 설정합니다").clicked() {
                            let target = if target_mod == 99 { None } else { Some(target_mod) };
                            self.zero_software_offsets(target);
                        }
                        if ui.button("🔄 기본값 리셋 (Gain 1.0, Offset 0.0)").clicked() {
                            if target_mod == 99 {
                                for i in 0..128 {
                                    if i < self.channel_gains.len() {
                                        self.channel_gains[i] = 1.0;
                                    }
                                    if i < self.channel_offsets.len() {
                                        self.channel_offsets[i] = 0.0;
                                    }
                                }
                            } else {
                                for ch_local in 0..16 {
                                    let global_ch = target_mod * 16 + ch_local;
                                    if global_ch < self.channel_gains.len() {
                                        self.channel_gains[global_ch] = 1.0;
                                    }
                                    if global_ch < self.channel_offsets.len() {
                                        self.channel_offsets[global_ch] = 0.0;
                                    }
                                }
                            }
                            self.save_config();
                        }
                    });
                });

                // 2. Module Selector Tabs
                ui.horizontal(|ui| {
                    ui.label(RichText::new("보기 모듈 필터:").strong());
                    if ui.selectable_label(self.channel_name_selected_mod == 99, "모든 모듈 (전체)").clicked() {
                        self.channel_name_selected_mod = 99;
                    }
                    let num_mods = self.current_module_count();
                    for m in 0..num_mods.min(8) {
                        let label = format!("모듈 {}", m);
                        if ui.selectable_label(self.channel_name_selected_mod == m, label).clicked() {
                            self.channel_name_selected_mod = m;
                        }
                    }
                });

                ui.separator();

                // 3. Scrollable Channel Grid with Gain/Offset (Dynamically fills available window height)
                let mut changed = false;
                let avail_height = (ui.available_height() - 48.0).max(180.0);
                egui::ScrollArea::vertical()
                    .id_salt("channel_names_scroll")
                    .max_height(avail_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        egui::Grid::new("channel_names_grid")
                            .striped(true)
                            .min_col_width(55.0)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("채널").strong());
                                ui.label(RichText::new("모듈").strong());
                                ui.label(RichText::new("채널 식별 이름").strong());
                                ui.label(RichText::new("Gain (이득)").strong().color(Color32::from_rgb(0, 229, 255)));
                                ui.label(RichText::new("Offset (오프셋)").strong().color(Color32::from_rgb(0, 229, 255)));
                                ui.label(RichText::new("원시 측정값 (Raw)").strong());
                                ui.label(RichText::new("계산된 값 (Calc)").strong().color(Color32::from_rgb(0, 230, 118)));
                                ui.label(RichText::new("작업").strong());
                                ui.end_row();

                                let start_ch = if self.channel_name_selected_mod == 99 { 0 } else { self.channel_name_selected_mod * 16 };
                                let end_ch = if self.channel_name_selected_mod == 99 { 128 } else { (self.channel_name_selected_mod + 1) * 16 };

                                for ch in start_ch..end_ch.min(self.channel_names.len()) {
                                    let m = ch / 16;
                                    let color = CHANNEL_COLORS[ch % 16];
                                    ui.colored_label(color, RichText::new(format!("Ch {:02}", ch + 1)).strong());
                                    ui.label(format!("Mod {}", m));

                                    if ui.add(
                                        egui::TextEdit::singleline(&mut self.channel_names[ch])
                                            .desired_width(160.0)
                                            .hint_text("이름 없음 (기본값)"),
                                    ).changed() {
                                        changed = true;
                                    }

                                    let mut gain = self.channel_gains.get(ch).copied().unwrap_or(1.0);
                                    if ui.add(
                                        egui::DragValue::new(&mut gain)
                                            .speed(0.001)
                                            .range(-100000.0..=100000.0)
                                            .custom_formatter(|n, _| format!("{:.4}", n))
                                    ).changed() {
                                        if ch < self.channel_gains.len() {
                                            self.channel_gains[ch] = gain;
                                            changed = true;
                                        }
                                    }

                                    let mut offset = self.channel_offsets.get(ch).copied().unwrap_or(0.0);
                                    if ui.add(
                                        egui::DragValue::new(&mut offset)
                                            .speed(0.001)
                                            .range(-100000.0..=100000.0)
                                            .custom_formatter(|n, _| format!("{:.4}", n))
                                    ).changed() {
                                        if ch < self.channel_offsets.len() {
                                            self.channel_offsets[ch] = offset;
                                            changed = true;
                                        }
                                    }

                                    let raw_val = self.channel_stats.get(ch).map(|s| s.raw_current).unwrap_or(0.0);
                                    let calc_val = self.channel_stats.get(ch).map(|s| s.calc_current).unwrap_or(0.0);
                                    ui.label(format!("{:6.3} PSI", raw_val));
                                    ui.colored_label(Color32::from_rgb(0, 230, 118), format!("{:6.3}", calc_val));

                                    ui.horizontal(|ui| {
                                        if ui.small_button("⚡ 영점").on_hover_text("이 채널의 현재 압력을 0.000 PSI로 오프셋 자동 계산").clicked() {
                                            self.zero_single_channel_offset(ch);
                                            changed = true;
                                        }
                                        if ui.small_button("초기화").on_hover_text("이름 비우기 및 Gain 1.0, Offset 0.0 리셋").clicked() {
                                            self.channel_names[ch].clear();
                                            if ch < self.channel_gains.len() {
                                                self.channel_gains[ch] = 1.0;
                                            }
                                            if ch < self.channel_offsets.len() {
                                                self.channel_offsets[ch] = 0.0;
                                            }
                                            changed = true;
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    });

                if changed {
                    self.save_config();
                }

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button(RichText::new("✔ 저장 및 닫기 (Save & Close)").strong().color(Color32::from_rgb(0, 230, 118))).clicked() {
                        self.save_config();
                        self.show_channel_names_modal = false;
                    }
                    if ui.button("닫기 (Close)").clicked() {
                        self.show_channel_names_modal = false;
                    }
                });
            });
        if !is_open {
            self.show_channel_names_modal = false;
        }
    }

    fn render_device_manager_window(&mut self, ctx: &egui::Context) {
        let mut is_open = self.show_device_manager_modal;
        egui::Window::new(RichText::new("DSA5000 장치 관리 및 SSEP 역할 설정 (Device & SSEP Manager)").strong().color(Color32::from_rgb(0, 229, 255)))
            .open(&mut is_open)
            .collapsible(false)
            .resizable(true)
            .min_width(980.0)
            .min_height(420.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.heading(RichText::new("DSA5000 다중 장치 목록 및 SSEP 역할 명시적 지정").color(Color32::from_rgb(0, 229, 255)));
                ui.label("네트워크 상의 각 DSA5000 모듈에 대해 Controller(2), Responder(3, 4), Auto(0), Standalone(1) 역할을 명시적으로 지정하고 하드웨어 Flash EEPROM에 영구 저장(SAVE)합니다.");
                ui.label(RichText::new("💡 매뉴얼 규격: SSEP 2=강제 Controller(마스터), SSEP 3=강제 Middle Responder(중간 슬레이브), SSEP 4=강제 Last Responder(끝단 슬레이브), SSEP 0=자동 할당(Auto), SSEP 1=스탠드얼론 강제").small().color(Color32::from_rgb(255, 234, 0)));
                ui.separator();

                let mut to_remove = None;
                let mut changed = false;
                let mut action_apply_save: Option<(usize, crate::protocol::SsepAssignment)> = None;
                let mut action_query: Option<usize> = None;
                let is_connected = self.is_connected;
                let num_devices = self.standalone_devices.len();

                let avail_h = (ui.available_height() - 90.0).max(180.0);
                egui::ScrollArea::vertical().id_salt("dev_mgr_scroll").max_height(avail_h).auto_shrink([false; 2]).show(ui, |ui| {
                    egui::Grid::new("device_manager_grid")
                        .striped(true)
                        .min_col_width(70.0)
                        .spacing([10.0, 8.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("모듈/채널").strong());
                            ui.label(RichText::new("IP 주소 (Host)").strong());
                            ui.label(RichText::new("Cmd").strong());
                            ui.label(RichText::new("Data").strong());
                            ui.label(RichText::new("장치 라벨/별칭").strong());
                            ui.label(RichText::new("SSEP 역할 명시적 지정 (Role)").strong().color(Color32::from_rgb(0, 229, 255)));
                            ui.label(RichText::new("통신 상태").strong());
                            ui.label(RichText::new("하드웨어 전송 & SAVE").strong().color(Color32::from_rgb(0, 230, 118)));
                            ui.label(RichText::new("삭제").strong());
                            ui.end_row();

                            for i in 0..num_devices {
                                let dev = &mut self.standalone_devices[i];
                                let ch_range = format!("Ch {:02}..{:02}", i * 16 + 1, (i + 1) * 16);
                                ui.colored_label(Color32::from_rgb(0, 229, 255), format!("Mod {} ({})", i, ch_range));

                                if ui.add(egui::TextEdit::singleline(&mut dev.host).desired_width(120.0)).changed() {
                                    changed = true;
                                }

                                let mut cp_str = dev.cmd_port.to_string();
                                if ui.add(egui::TextEdit::singleline(&mut cp_str).desired_width(45.0)).changed() {
                                    if let Ok(p) = cp_str.parse() {
                                        dev.cmd_port = p;
                                        changed = true;
                                    }
                                }

                                let mut dp_str = dev.data_port.to_string();
                                if ui.add(egui::TextEdit::singleline(&mut dp_str).desired_width(45.0)).changed() {
                                    if let Ok(p) = dp_str.parse() {
                                        dev.data_port = p;
                                        changed = true;
                                    }
                                }

                                if ui.add(egui::TextEdit::singleline(&mut dev.label).desired_width(110.0).hint_text("예: Wing_Tip")).changed() {
                                    changed = true;
                                }

                                // Explicit SSEP Role Selector
                                egui::ComboBox::from_id_salt(format!("ssep_role_combo_{}", i))
                                    .selected_text(dev.ssep_role.label())
                                    .show_ui(ui, |ui| {
                                        for r in [
                                            crate::protocol::SsepAssignment::Controller,
                                            crate::protocol::SsepAssignment::MiddleResponder,
                                            crate::protocol::SsepAssignment::LastResponder,
                                            crate::protocol::SsepAssignment::Auto,
                                            crate::protocol::SsepAssignment::Standalone,
                                        ] {
                                            if ui.selectable_value(&mut dev.ssep_role, r, r.label()).clicked() {
                                                changed = true;
                                            }
                                        }
                                    });

                                let status_text = match self.device_test_status.get(i).copied().flatten() {
                                    Some(true) => RichText::new("● 통신 가능").color(Color32::from_rgb(0, 230, 118)),
                                    Some(false) => RichText::new("● 응답 없음").color(Color32::from_rgb(255, 60, 60)),
                                    None => RichText::new("○ 미확인").color(Color32::GRAY),
                                };
                                ui.label(status_text);

                                ui.horizontal(|ui| {
                                    let role_val = dev.ssep_role;
                                    if ui.button(RichText::new("✔ 적용 & SAVE").strong().color(Color32::from_rgb(0, 230, 118)))
                                        .on_hover_text(format!("모듈 {}에 SSEP {} 명령을 전송하고 Flash EEPROM에 영구 저장(SAVE)합니다.", i, role_val.to_u8()))
                                        .clicked()
                                    {
                                        action_apply_save = Some((i, role_val));
                                    }
                                    if is_connected {
                                        if ui.small_button("🔍 조회").on_hover_text("현재 장비의 SSEP 모드 확인").clicked() {
                                            action_query = Some(i);
                                        }
                                    }
                                });

                                if num_devices > 1 {
                                    if ui.small_button("🗑 삭제").clicked() {
                                        to_remove = Some(i);
                                    }
                                } else {
                                    ui.label("-");
                                }
                                ui.end_row();
                            }
                        });
                });

                if let Some((idx, r)) = action_apply_save {
                    self.handle_apply_and_save_ssep_role(idx, r);
                }

                if let Some(idx) = action_query {
                    self.handle_query_ssep(Some(idx));
                }

                if let Some(idx) = to_remove {
                    self.standalone_devices.remove(idx);
                    changed = true;
                }

                if changed {
                    self.sync_host_string_from_devices();
                    self.save_config();
                }

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button(RichText::new("➕ 새 장치 추가").strong().color(Color32::from_rgb(0, 229, 255))).clicked() {
                        let new_idx = self.standalone_devices.len();
                        let base_ip = self.standalone_devices.last().map(|d| d.host.clone()).unwrap_or_else(|| "191.30.130.5".into());
                        let next_ip = if let Some(last_dot) = base_ip.rfind('.') {
                            if let Ok(last_octet) = base_ip[last_dot + 1..].parse::<u32>() {
                                format!("{}.{}", &base_ip[..last_dot], last_octet + 1)
                            } else {
                                format!("{}_{}", base_ip, new_idx + 1)
                            }
                        } else {
                            format!("{}_{}", base_ip, new_idx + 1)
                        };
                        self.standalone_devices.push(crate::client::DeviceEndpoint {
                            host: next_ip,
                            cmd_port: self.cmd_port,
                            data_port: self.data_port,
                            label: format!("장치 {} (Ch {}-{})", new_idx + 1, new_idx * 16 + 1, (new_idx + 1) * 16),
                            ssep_role: crate::protocol::SsepAssignment::LastResponder,
                        });
                        self.sync_host_string_from_devices();
                        self.save_config();
                    }

                    if ui.button("⚡ 연결 테스트 (Ping)").clicked() {
                        self.device_test_status = vec![None; self.standalone_devices.len()];
                        for (i, d) in self.standalone_devices.iter().enumerate() {
                            let addr_str = format!("{}:{}", d.host, d.cmd_port);
                            let ok = if let Ok(addrs) = addr_str.to_socket_addrs() {
                                if let Some(addr) = addrs.into_iter().next() {
                                    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            self.device_test_status[i] = Some(ok);
                        }
                    }

                    if self.is_connected && self.standalone_devices.len() > 1 {
                        if ui.button(RichText::new("🚀 지정된 SSEP 역할 일괄 적용 & SAVE").strong().color(Color32::from_rgb(0, 229, 255)))
                            .on_hover_text("사용자가 테이블에서 지정한 각 장치별 Controller/Responder 역할을 장비에 일괄 전송하고 Flash EEPROM에 영구 저장합니다.")
                            .clicked()
                        {
                            self.handle_apply_designated_ssep_roles();
                        }

                        if ui.button(RichText::new("⚡ 기본 순환 배치 (0번=Controller, 끝단=Last Responder) & SAVE").color(Color32::LIGHT_GRAY))
                            .on_hover_text("모듈 0번은 SSEP 2(Controller), 마지막 모듈은 SSEP 4(Last Responder)로 자동 배정하고 SAVE합니다.")
                            .clicked()
                        {
                            self.handle_configure_ssep_chain();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("✔ 닫기 (Close)").strong().color(Color32::from_rgb(0, 230, 118))).clicked() {
                            self.sync_host_string_from_devices();
                            self.save_config();
                            self.show_device_manager_modal = false;
                        }
                    });
                });
            });

        if !is_open {
            self.show_device_manager_modal = false;
        }
    }

    fn render_bottom_status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("Model: {}", self.system_info.model));
            ui.separator();
            ui.label(format!("S/N: {}", self.system_info.serial_number));
            ui.separator();
            let role_label = if self.operating_mode == OperatingMode::Standalone {
                "Mode: Standalone (16 Ch)".to_string()
            } else {
                format!("SSEP Role: {} ({} Responders)", self.ssep_state.unit_role, self.ssep_state.num_responders)
            };
            ui.label(role_label);
            ui.separator();
            ui.colored_label(Color32::from_rgb(255, 234, 0), format!("Rate: {} Hz", self.scan_rate_hz));
            ui.separator();
            ui.colored_label(Color32::from_rgb(0, 229, 255), format!("FPS: {}", self.client_stats.current_fps));
            ui.separator();
            let drop_color = if self.client_stats.dropped_frames > 0 {
                Color32::from_rgb(255, 60, 60)
            } else {
                Color32::from_rgb(0, 230, 118)
            };
            ui.colored_label(drop_color, format!("Loss: {}", self.client_stats.dropped_frames));
            ui.separator();
            ui.label(format!("Unit: {}", self.engineering_units));
            ui.separator();
            if self.operating_mode == OperatingMode::Ssep {
                ui.label(format!("Link In: {}, Out: {}", self.ssep_state.link_in, self.ssep_state.link_out));
                ui.separator();
            }
            if self.is_calibrating {
                ui.colored_label(
                    Color32::from_rgb(255, 234, 0),
                    RichText::new(format!("⚡ CALZ 교정 진행 중 ({}%)", self.calz_progress_pct)).strong(),
                );
                ui.separator();
            }
            if self.is_command_pending {
                ui.colored_label(
                    Color32::from_rgb(255, 234, 0),
                    RichText::new(format!("⏳ {}", self.command_pending_desc)).strong(),
                );
                ui.separator();
            }
            if self.logger.is_recording {
                ui.colored_label(
                    Color32::from_rgb(255, 60, 60),
                    format!("● CSV RECORDING ({} frames)", self.logger.recorded_frames),
                );
            } else {
                ui.label("Logger: Ready");
            }
            ui.separator();
            let (ptp_str, jitter_str) = self.format_ptp_display();
            ui.colored_label(Color32::from_rgb(170, 255, 170), RichText::new(format!("⏱ {} | {}", ptp_str, jitter_str)).monospace());
        });
    }

    fn render_oscilloscope_tab(&mut self, ui: &mut egui::Ui) {
        let num_mods = self.current_module_count();

        // 1. Module Filter Controls (Row 1 - Clean horizontal row)
        ui.horizontal(|ui| {
            if self.operating_mode == OperatingMode::Standalone && num_mods <= 1 {
                ui.label(RichText::new("동작 모드:").strong().color(Color32::from_rgb(0, 229, 255)));
                ui.label(RichText::new("스탠드얼론 단독 (Ch 01 ~ Ch 16)").strong().color(Color32::WHITE));
            } else {
                let lbl_prefix = if self.operating_mode == OperatingMode::Standalone { "장치/모듈 필터:" } else { "모듈 필터:" };
                ui.label(RichText::new(lbl_prefix).strong().color(Color32::from_rgb(0, 229, 255)));

                if ui.small_button("전체 선택").clicked() {
                    self.selected_modules.fill(true);
                    self.save_config();
                }
                if ui.small_button("전체 해제").clicked() {
                    self.selected_modules.fill(false);
                    self.save_config();
                }

                for m in 0..num_mods {
                    let is_sel = self.selected_modules.get(m).copied().unwrap_or(true);
                    let lbl = format!("모듈 {} (Ch {}-{})", m, m * 16 + 1, (m + 1) * 16);
                    let btn = ui.selectable_label(is_sel, lbl);
                    if btn.clicked() {
                        if m < self.selected_modules.len() {
                            self.selected_modules[m] = !is_sel;
                            self.save_config();
                        }
                    }
                }
            }
        });

        // 2. Time Window & Oscilloscope Display Controls (Row 2 - Clean horizontal row)
        ui.horizontal(|ui| {
            ui.label(RichText::new("시간 범위:").strong());
            let time_presets = [1.0, 2.0, 5.0, 10.0, 30.0, 60.0];
            for p in time_presets {
                let lbl = format!("{:.0}s", p);
                if ui.selectable_label((self.time_window_sec - p).abs() < 0.1, lbl).clicked() {
                    self.time_window_sec = p;
                    self.save_config();
                }
            }
            if ui.add(egui::Slider::new(&mut self.time_window_sec, 1.0..=60.0).suffix("s")).changed() {
                self.save_config();
            }

            ui.separator();

            if ui.checkbox(&mut self.auto_scale_y, "Y축 자동 스케일").changed() {
                self.save_config();
            }
            if !self.auto_scale_y {
                ui.label("Y Min:");
                if ui.add(egui::DragValue::new(&mut self.y_min).speed(0.1)).changed() {
                    self.save_config();
                }
                ui.label("Y Max:");
                if ui.add(egui::DragValue::new(&mut self.y_max).speed(0.1)).changed() {
                    self.save_config();
                }
            }

            if ui.checkbox(&mut self.show_internal_legend, "플롯 내부 범례").changed() {
                self.save_config();
            }

            if ui.button(if self.is_paused { "▶ 계속 (Resume)" } else { "⏸ 일시 정지" }).clicked() {
                self.is_paused = !self.is_paused;
                if !self.is_paused {
                    self.cursor_pos = None;
                }
            }

            if ui.button("🗑 데이터 버퍼 초기화").clicked() {
                self.data_history.clear();
            }
        });

        // 3. Zoom Controls & Plot Appearance (Row 3 - Zoom & Cursor Toolbar)
        ui.horizontal(|ui| {
            ui.label(RichText::new("🔍 줌 제어:").strong().color(Color32::from_rgb(0, 229, 255)));

            // Mouse Scroll Zoom Axis Selector (전체 / X축 / Y축)
            ui.label(RichText::new("휠 줌 축:").strong());
            if ui.selectable_value(&mut self.zoom_target_axis, ZoomTargetAxis::All, "전체(X+Y)").on_hover_text("마우스 휠 스크롤 시 X축(시간)과 Y축(압력)을 동시에 확대/축소합니다").clicked() {
                self.save_config();
            }
            if ui.selectable_value(&mut self.zoom_target_axis, ZoomTargetAxis::X, "X축(시간)").on_hover_text("마우스 휠 스크롤 시 X축(시간 범위)만 확대/축소합니다").clicked() {
                self.save_config();
            }
            if ui.selectable_value(&mut self.zoom_target_axis, ZoomTargetAxis::Y, "Y축(압력)").on_hover_text("마우스 휠 스크롤 시 Y축(압력 스케일)만 확대/축소합니다").clicked() {
                self.save_config();
            }

            if ui.button(RichText::new("🔄 줌 리셋").color(Color32::from_rgb(255, 234, 0))).on_hover_text("줌 배율을 1.0x(기본 자동 맞춤)으로 초기화합니다").clicked() {
                self.y_zoom_factor = 1.0;
                self.time_window_sec = 5.0;
                self.auto_scale_y = true;
                self.save_config();
            }

            ui.separator();

            // Plot Line Thickness (Thinner default: 1.0px)
            ui.label("선 두께:");
            if ui.add(egui::DragValue::new(&mut self.plot_line_width).speed(0.1).range(0.5..=4.0).custom_formatter(|v, _| format!("{:.1}px", v))).on_hover_text("플롯 곡선 선 두께 조절 (기본 1.0px)").changed() {
                self.save_config();
            }
            let width_presets = [0.8f32, 1.0f32, 1.2f32, 1.8f32];
            for wp in width_presets {
                if ui.selectable_label((self.plot_line_width - wp).abs() < 0.05, format!("{:.1}", wp)).clicked() {
                    self.plot_line_width = wp;
                    self.save_config();
                }
            }

            ui.separator();

            // Cursor Value Display Toggle
            if ui.checkbox(&mut self.show_cursor_info, "📍 커서 위치 계측값 표시").changed() {
                self.save_config();
            }
        });

        ui.separator();

        // 4. Adaptive Time Window & Zero-Gap Point Interpolation
        let latest_t = self.data_history.last().map(|(t, _)| *t).unwrap_or(0.0);
        let first_t = self.data_history.first().map(|(t, _)| *t).unwrap_or(0.0);
        let data_duration = (latest_t - first_t).max(0.0);

        // Dynamically adapt view span: if data is accumulating, fill 100% from left to right;
        // once full time window is reached, lock to time_window_sec and scroll in real-time.
        let view_span = if data_duration >= self.time_window_sec {
            self.time_window_sec
        } else {
            data_duration.clamp(0.2, self.time_window_sec)
        };
        let min_t = latest_t - view_span;

        let avail_w = ui.available_width();
        let avail_h = ui.available_height();
        let sidebar_w = 240.0;
        let plot_width = (avail_w - sidebar_w - 35.0).max(350.0);
        let plot_height = (avail_h - 15.0).max(250.0);

        let mut channel_lines = Vec::new();
        let mut y_data_min = f64::MAX;
        let mut y_data_max = f64::MIN;

        for m in 0..num_mods {
            if !self.selected_modules.get(m).copied().unwrap_or(true) {
                continue;
            }
            for ch_local in 0..16 {
                let ch = (m * 16) + ch_local;
                if !self.active_channels.get(ch).copied().unwrap_or(true) {
                    continue;
                }

                let mut raw_pts = Vec::new();
                for (t, p_arr) in &self.data_history {
                    if *t >= min_t {
                        if let Some(&p) = p_arr.get(ch) {
                            let rel_x = *t - latest_t;
                            let p_f64 = p as f64;
                            raw_pts.push([rel_x, p_f64]);

                            if p_f64 < y_data_min {
                                y_data_min = p_f64;
                            }
                            if p_f64 > y_data_max {
                                y_data_max = p_f64;
                            }
                        }
                    }
                }

                // Dynamic Decimation: Preserve 100% raw samples up to 8,000 points (prevents high-frequency trembling at 5s+ scale).
                // Above 8,000 points, uses jitter-free Min-Max screen buckets.
                let pts = if raw_pts.len() > 8000 {
                    let target_buckets = 2000;
                    let bucket_dt = view_span / target_buckets as f64;
                    let mut buckets: Vec<Option<(f64, f64, f64, f64)>> = vec![None; target_buckets];

                    for pt in raw_pts {
                        let x = pt[0];
                        let y = pt[1];
                        let b_idx = (((x + view_span) / bucket_dt).floor() as usize).min(target_buckets - 1);
                        match &mut buckets[b_idx] {
                            None => {
                                buckets[b_idx] = Some((x, y, x, y));
                            }
                            Some(b) => {
                                if y < b.1 {
                                    b.0 = x;
                                    b.1 = y;
                                }
                                if y > b.3 {
                                    b.2 = x;
                                    b.3 = y;
                                }
                            }
                        }
                    }

                    let mut decimated = Vec::with_capacity(target_buckets * 2);
                    for b in buckets.into_iter().flatten() {
                        if (b.0 - b.2).abs() < 1e-6 {
                            decimated.push([b.0, b.1]);
                        } else if b.0 < b.2 {
                            decimated.push([b.0, b.1]);
                            decimated.push([b.2, b.3]);
                        } else {
                            decimated.push([b.2, b.3]);
                            decimated.push([b.0, b.1]);
                        }
                    }
                    decimated
                } else {
                    raw_pts
                };

                let color = CHANNEL_COLORS[ch % 16];
                let custom = self.get_channel_custom_name(ch);
                let line_name = if !custom.is_empty() {
                    format!("Ch{:02} [{}] (M{})", ch + 1, custom, m)
                } else {
                    format!("Ch{:02} (M{})", ch + 1, m)
                };
                let line = Line::new(PlotPoints::new(pts))
                    .name(line_name)
                    .color(color)
                    .stroke(Stroke::new(self.plot_line_width, color));
                channel_lines.push(line);
            }
        }

        // Compute Y-axis bounds dynamically with zoom multiplier and safe headroom
        let (y_view_min, y_view_max) = if self.auto_scale_y {
            if y_data_min <= y_data_max {
                let diff = y_data_max - y_data_min;
                let span = if diff > 1e-5 { diff } else { 0.005 };
                let center = (y_data_min + y_data_max) / 2.0;
                let half_span = (span / 2.0) / self.y_zoom_factor.max(0.01);
                let margin = half_span * 0.06; // 6% margin so waveform fits comfortably and never exceeds plot area
                (center - half_span - margin, center + half_span + margin)
            } else {
                let center = 0.225;
                let half_span = 0.275 / self.y_zoom_factor.max(0.01);
                (center - half_span, center + half_span)
            }
        } else {
            let center = (self.y_min + self.y_max) / 2.0;
            let half_span = ((self.y_max - self.y_min) / 2.0) / self.y_zoom_factor.max(0.01);
            (center - half_span, center + half_span)
        };

        ui.horizontal(|ui| {
            // Left: Real-Time Oscilloscope Plot
            ui.allocate_ui_with_layout(
                egui::vec2(plot_width, plot_height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    let mut plot = Plot::new("ssep_oscilloscope")
                        .width(plot_width)
                        .height(plot_height)
                        .show_x(true)
                        .show_y(true)
                        .allow_zoom(false)
                        .allow_drag(false)
                        .allow_scroll(false)
                        .allow_boxed_zoom(false)
                        .allow_double_click_reset(false)
                        .x_axis_formatter(|mark, _range| format!("{:.1}s", mark.value));

                    if self.show_internal_legend {
                        plot = plot.legend(Legend::default().position(Corner::RightTop));
                    }

                    let plot_response = plot.show(ui, |plot_ui| {
                        // Lock plot bounds exactly to span and headroom
                        plot_ui.set_plot_bounds(egui_plot::PlotBounds::from_min_max(
                            [-view_span, y_view_min],
                            [0.0, y_view_max],
                        ));

                        for line in channel_lines {
                            plot_ui.line(line);
                        }

                        // Cursor tracking & real-time crosshair (subtle cyan, no yellow text)
                        if self.show_cursor_info {
                            if let Some(coord) = plot_ui.pointer_coordinate() {
                                self.cursor_pos = Some((coord.x, coord.y));
                            } else if self.is_scanning && !self.is_paused {
                                self.cursor_pos = None;
                            }

                            if let Some((cx, cy)) = self.cursor_pos {
                                let crosshair_stroke = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0, 229, 255, 140));
                                plot_ui.vline(VLine::new(cx).stroke(crosshair_stroke));
                                plot_ui.hline(HLine::new(cy).stroke(crosshair_stroke));
                            }
                        }
                    });

                    // Mouse scroll wheel zoom handling: zooms selected axis within data range
                    if plot_response.response.hovered() {
                        let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
                        if scroll_y.abs() > 0.01 {
                            let zoom_in = scroll_y > 0.0;
                            // X-Axis Zoom (Clamped strictly within accumulated data duration)
                            if self.zoom_target_axis == ZoomTargetAxis::All || self.zoom_target_axis == ZoomTargetAxis::X {
                                if zoom_in {
                                    self.time_window_sec = (self.time_window_sec * 0.85).max(0.05);
                                } else {
                                    let max_span = data_duration.max(1.0).min(120.0);
                                    self.time_window_sec = (self.time_window_sec * 1.15).min(max_span);
                                }
                                self.save_config();
                            }
                            // Y-Axis Zoom
                            if self.zoom_target_axis == ZoomTargetAxis::All || self.zoom_target_axis == ZoomTargetAxis::Y {
                                if zoom_in {
                                    self.y_zoom_factor = (self.y_zoom_factor * 1.15).min(50.0);
                                } else {
                                    self.y_zoom_factor = (self.y_zoom_factor * 0.85).max(0.1);
                                }
                                self.save_config();
                            }
                        }
                    }
                },
            );

            ui.separator();

            // Determine stopped state and cursor frame data for vertical readout
            let is_stopped = !self.is_scanning || self.is_paused;
            let cursor_frame: Option<(f64, Vec<f32>)> = if is_stopped && self.show_cursor_info {
                if let Some((cx, _)) = self.cursor_pos {
                    let target_t = latest_t + cx;
                    self.data_history.iter().min_by(|(t1, _), (t2, _)| {
                        (t1 - target_t).abs().partial_cmp(&(t2 - target_t).abs()).unwrap_or(std::cmp::Ordering::Equal)
                    }).map(|(t, vals)| (*t - latest_t, vals.clone()))
                } else {
                    None
                }
            } else {
                None
            };

            // Right: Scrollable Legend & Channel Selector Panel (Vertical Readout)
            ui.vertical(|ui| {
                ui.set_width(sidebar_w);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("채널 가시성/범례").strong());
                    if ui.small_button("켬").clicked() {
                        self.active_channels.fill(true);
                    }
                    if ui.small_button("끔").clicked() {
                        self.active_channels.fill(false);
                    }
                    if ui.small_button("🏷 이름").clicked() {
                        self.show_channel_names_modal = true;
                    }
                });

                if let Some((rel_t, _)) = cursor_frame {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("📍 커서:").strong().size(12.0).color(Color32::from_rgb(0, 229, 255)));
                        ui.label(RichText::new(format!("{:+.3} s", rel_t)).strong().size(12.0).color(Color32::WHITE));
                        if let Some((_, cy)) = self.cursor_pos {
                            ui.label(RichText::new(format!("({:+.2})", cy)).size(11.0).color(Color32::GRAY));
                        }
                    });
                } else if is_stopped {
                    ui.label(RichText::new("⏸ 정지 (그래프 위 커서로 시간 탐색)").size(11.0).color(Color32::GRAY));
                }

                ui.separator();

                egui::ScrollArea::vertical()
                    .id_salt("legend_sidebar_scroll")
                    .max_height(plot_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for m in 0..num_mods {
                            if !self.selected_modules.get(m).copied().unwrap_or(true) {
                                continue;
                            }

                            ui.label(RichText::new(format!("■ 모듈 {}", m)).strong().size(13.0).color(Color32::from_rgb(0, 229, 255)));

                            for ch_local in 0..16 {
                                let ch = (m * 16) + ch_local;
                                let color = CHANNEL_COLORS[ch % 16];
                                let is_act = self.active_channels.get(ch).copied().unwrap_or(true);
                                let (disp_val, disp_color) = if let Some((_, vals)) = &cursor_frame {
                                    if let Some(&p) = vals.get(ch) {
                                        (p, Color32::from_rgb(0, 229, 255))
                                    } else {
                                        (self.channel_stats.get(ch).map(|s| s.current).unwrap_or(0.0), Color32::WHITE)
                                    }
                                } else {
                                    (self.channel_stats.get(ch).map(|s| s.current).unwrap_or(0.0), Color32::WHITE)
                                };

                                ui.horizontal(|ui| {
                                    let mut checked = is_act;
                                    if ui.checkbox(&mut checked, "").changed() {
                                        if ch < self.active_channels.len() {
                                            self.active_channels[ch] = checked;
                                        }
                                    }
                                    let tag = format!("Ch{:02}", ch + 1);
                                    let custom = self.get_channel_custom_name(ch);
                                    if !custom.is_empty() {
                                        ui.colored_label(color, RichText::new(&tag).strong().size(12.0));
                                        ui.label(RichText::new(custom).size(10.5).color(Color32::from_rgb(170, 225, 255)));
                                    } else {
                                        ui.colored_label(color, RichText::new(&tag).strong().size(13.0));
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add_space(6.0); // 6px safety margin so number never touches right border/scrollbar
                                        ui.label(RichText::new(format!("{:6.2}", disp_val)).size(15.0).strong().color(disp_color));
                                    });
                                });
                            }
                            ui.separator();
                        }
                    });
            });
        });
    }

    /// Checkerboard / Grid Layout for Bar Meters (바둑판 식 배치)
    fn render_bar_meters_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("실시간 채널 압력 바미터 (바둑판 매트릭스)").heading());
            ui.separator();

            ui.label("배치 열 수:");
            if ui.selectable_label(self.bar_meter_cols == 8, "8-열 바둑판 (8 Ch)").clicked() {
                self.bar_meter_cols = 8;
                self.save_config();
            }
            if ui.selectable_label(self.bar_meter_cols == 16, "16-열 바둑판 (16 Ch)").clicked() {
                self.bar_meter_cols = 16;
                self.save_config();
            }
            if ui.selectable_label(self.bar_meter_cols == 4, "4-열 바둑판 (4 Ch)").clicked() {
                self.bar_meter_cols = 4;
                self.save_config();
            }

            ui.separator();

            ui.label("풀 스케일:");
            let scale_presets = [15.0f32, 30.0f32, 50.0f32, 100.0f32];
            for sc in scale_presets {
                let lbl = format!("{:.0} PSI", sc);
                if ui.selectable_label((self.bar_meter_max_psi - sc).abs() < 0.1, lbl).clicked() {
                    self.bar_meter_max_psi = sc;
                    self.save_config();
                }
            }

            ui.separator();

            if ui.button(RichText::new("🏷 채널 이름 설정").strong().color(Color32::from_rgb(0, 229, 255))).clicked() {
                self.show_channel_names_modal = true;
            }
        });
        ui.separator();

        let num_mods = self.current_module_count();
        let max_psi = if self.bar_meter_max_psi <= 0.0 { 15.0 } else { self.bar_meter_max_psi };
        let cols = self.bar_meter_cols.clamp(2, 16);

        egui::ScrollArea::vertical().id_salt("bar_meters_grid_scroll").show(ui, |ui| {
            // Reserve a safe margin for scrollbar and container padding so columns NEVER overflow
            let avail_w = (ui.available_width() - 8.0).max(200.0);

            for m in 0..num_mods {
                if !self.selected_modules.get(m).copied().unwrap_or(true) {
                    continue;
                }

                let mod_sn = self
                    .ssep_state
                    .modules
                    .get(m)
                    .map(|info| info.serial_number)
                    .unwrap_or(50421 + m as u32);
                let role = if self.operating_mode == OperatingMode::Standalone {
                    "Standalone"
                } else if m == 0 {
                    "Controller"
                } else {
                    "Responder"
                };

                let mod_frame = egui::Frame::none()
                    .fill(Color32::from_rgb(18, 22, 28))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 45, 56)))
                    .rounding(6.0)
                    .inner_margin(egui::Margin::symmetric(6.0, 6.0));

                mod_frame.show(ui, |ui| {
                    ui.set_width(avail_w);
                    ui.label(RichText::new(format!("모듈 {} [{}: S/N {}] - 16채널 바둑판 매트릭스", m, role, mod_sn))
                        .strong()
                        .color(Color32::from_rgb(0, 229, 255)));

                    let spacing_x = if cols > 8 { 3.0 } else { 6.0 };
                    let spacing_y = 6.0;

                    // Inner usable width inside module container (accounting for 12px frame padding)
                    let inner_w = (avail_w - 14.0).max(100.0);
                    let total_gaps = (cols as f32 - 1.0) * spacing_x;
                    // Exact mathematical division guarantee: cols * col_w + total_gaps <= inner_w
                    let col_w = ((inner_w - total_gaps) / (cols as f32)).floor().max(35.0);

                    egui::Grid::new(format!("grid_mod_{}", m))
                        .min_col_width(col_w)
                        .spacing([spacing_x, spacing_y])
                        .show(ui, |ui| {
                            for ch_local in 0..16 {
                                let global_ch = (m * 16) + ch_local;
                                let val = self.channel_stats.get(global_ch).map(|s| s.current).unwrap_or(0.0);
                                let temp = self.channel_stats.get(global_ch).map(|s| s.temp).unwrap_or(25.0);
                                let min_v = self.channel_stats.get(global_ch).map(|s| s.min).unwrap_or(val);
                                let max_v = self.channel_stats.get(global_ch).map(|s| s.max).unwrap_or(val);
                                let color = CHANNEL_COLORS[ch_local % 16];

                                // Precision Zero-Overflow Card Frame
                                let card_frame = egui::Frame::none()
                                    .fill(Color32::from_rgb(22, 26, 34))
                                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(45, 52, 64)))
                                    .rounding(4.0)
                                    .inner_margin(egui::Margin::symmetric(3.0, 4.0));

                                card_frame.show(ui, |ui| {
                                    // Internal width accounts for 6px (3px + 3px) inner margin so outer card is exactly col_w
                                    let content_w = (col_w - 6.0).max(20.0);
                                    ui.set_width(content_w);

                                    ui.vertical(|ui| {
                                        // Header: Ch XX tag + Custom Name + Temp
                                        let custom = self.get_channel_custom_name(global_ch);
                                        ui.horizontal(|ui| {
                                            ui.colored_label(color, RichText::new(format!("Ch{:02}", global_ch + 1)).strong().size(12.0));
                                            if !custom.is_empty() && col_w >= 75.0 {
                                                ui.label(RichText::new(custom).size(10.5).color(Color32::from_rgb(0, 229, 255)));
                                            }
                                            if col_w >= 65.0 {
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    let temp_str = if col_w < 85.0 { format!("{:.0}°C", temp) } else { format!("{:.1}°C", temp) };
                                                    ui.label(RichText::new(temp_str).size(10.0).color(Color32::GRAY));
                                                });
                                            }
                                        });

                                        // Big Numeric Value Display
                                        let val_size = if col_w < 75.0 { 15.0 } else if col_w < 120.0 { 18.5 } else { 22.0 };
                                        ui.label(RichText::new(format!("{:6.3}", val)).size(val_size).strong().color(Color32::WHITE));

                                        // Dynamic Width Vertical Bar Graphic Meter (Fills entire cell width!)
                                        let bar_w = (content_w - 2.0).max(15.0);
                                        let (rect, _) = ui.allocate_exact_size(Vec2::new(bar_w, 65.0), egui::Sense::hover());
                                        ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(16, 20, 28));

                                        let pct = (val / max_psi).clamp(0.0, 1.0);
                                        let fill_h = rect.height() * pct;
                                        let fill_rect = egui::Rect::from_min_max(
                                            egui::pos2(rect.min.x + 1.5, rect.max.y - fill_h),
                                            egui::pos2(rect.max.x - 1.5, rect.max.y),
                                        );
                                        ui.painter().rect_filled(fill_rect, 2.0, color);

                                        // Min / Max footer (Adapts precision to avoid overflowing column)
                                        let footer_str = if col_w < 85.0 {
                                            format!("L:{:.1} H:{:.1}", if min_v.is_finite() { min_v } else { 0.0 }, if max_v.is_finite() { max_v } else { 0.0 })
                                        } else {
                                            format!("L:{:.2} H:{:.2}", if min_v.is_finite() { min_v } else { 0.0 }, if max_v.is_finite() { max_v } else { 0.0 })
                                        };
                                        ui.label(RichText::new(footer_str).size(9.5).color(Color32::DARK_GRAY));
                                    });
                                });

                                if (ch_local + 1) % cols == 0 || ch_local == 15 {
                                    ui.end_row();
                                }
                            }
                        });
                });
                ui.add_space(4.0);
            }
        });
    }

    fn render_statistics_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("실시간 다채널 압력 통계 분석 (Gain / Offset 보정 계산)").heading());
            ui.separator();
            ui.label(RichText::new("보정 수식: 계산값 (Calc) = 원본 (Raw) × Gain + Offset").color(Color32::from_rgb(0, 229, 255)));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("⚡ 즉각 영점 (Tare)").color(Color32::from_rgb(0, 230, 118)))
                    .on_hover_text("현재 측정 압력을 기준으로 모든 채널을 즉시 0.000 PSI로 설정합니다.")
                    .clicked()
                {
                    self.handle_zero_tare(self.control_target);
                }
                if ui.button(RichText::new("🔄 통계 초기화 (Reset)").color(Color32::from_rgb(255, 100, 100)))
                    .on_hover_text("누적된 최소, 최대, 평균, 노이즈 통계 데이터를 0으로 초기화합니다.")
                    .clicked()
                {
                    for st in &mut self.channel_stats {
                        *st = ChannelStats::default();
                    }
                }
                if ui.button(RichText::new("🏷 Gain / Offset 설정 열기").color(Color32::from_rgb(0, 230, 118)))
                    .on_hover_text("채널 식별명 및 개별 Gain/Offset 보정값을 설정하는 창을 엽니다.")
                    .clicked()
                {
                    self.show_channel_names_modal = true;
                }
            });
        });
        ui.separator();

        let num_mods = self.current_module_count();

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("stats_table")
                .striped(true)
                .min_col_width(65.0)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("채널").strong());
                    ui.label(RichText::new("모듈").strong());
                    ui.label(RichText::new("Gain").strong().color(Color32::from_rgb(255, 234, 0)));
                    ui.label(RichText::new("Offset").strong().color(Color32::from_rgb(255, 234, 0)));
                    ui.label(RichText::new("원시값 (Raw)").strong());
                    ui.label(RichText::new("계산값 (Calc)").strong().color(Color32::from_rgb(0, 230, 118)));
                    ui.label(RichText::new("계산 평균 (Mean)").strong());
                    ui.label(RichText::new("계산 최소 (Min)").strong());
                    ui.label(RichText::new("계산 최대 (Max)").strong());
                    ui.label(RichText::new("P-P 노이즈").strong());
                    ui.label(RichText::new("RMS 변동").strong());
                    ui.label(RichText::new("모듈 온도").strong());
                    ui.end_row();

                    for m in 0..num_mods {
                        if !self.selected_modules.get(m).copied().unwrap_or(true) {
                            continue;
                        }

                        for ch_local in 0..16 {
                            let ch = (m * 16) + ch_local;
                            let st = self.channel_stats.get(ch).cloned().unwrap_or_default();
                            let gain = self.channel_gains.get(ch).copied().unwrap_or(1.0);
                            let offset = self.channel_offsets.get(ch).copied().unwrap_or(0.0);

                            let calc_val = st.calc_value(gain, offset);
                            let calc_mean = st.calc_mean(gain, offset);
                            let calc_min = st.calc_min(gain, offset);
                            let calc_max = st.calc_max(gain, offset);
                            let calc_pp = st.calc_pp(gain);
                            let calc_rms = st.calc_rms(gain);

                            let color = CHANNEL_COLORS[ch_local % 16];
                            let custom = self.get_channel_custom_name(ch);
                            let tag = if !custom.is_empty() {
                                format!("Ch {:02} [{}]", ch + 1, custom)
                            } else {
                                format!("Ch {:02}", ch + 1)
                            };
                            ui.colored_label(color, tag);
                            ui.label(format!("Mod {}", m));
                            ui.colored_label(Color32::from_rgb(255, 234, 0), format!("{:.4}", gain));
                            ui.colored_label(Color32::from_rgb(255, 234, 0), format!("{:.4}", offset));
                            ui.label(format!("{:.3} PSI", st.raw_current));
                            ui.colored_label(Color32::from_rgb(0, 230, 118), RichText::new(format!("{:.3}", calc_val)).strong());
                            ui.label(format!("{:.3}", calc_mean));
                            ui.label(format!("{:.3}", calc_min));
                            ui.label(format!("{:.3}", calc_max));
                            ui.label(format!("{:.3}", calc_pp));
                            ui.label(format!("{:.4}", calc_rms));
                            ui.label(format!("{:.1} °C", st.temp));
                            ui.end_row();
                        }
                    }
                });
        });
    }

    fn render_ssep_topology_tab(&mut self, ui: &mut egui::Ui) {
        if self.operating_mode == OperatingMode::Standalone {
            let mod_count = self.current_module_count();
            if mod_count <= 1 {
                ui.label(RichText::new("DSA5000 스탠드얼론 (단독 16채널) 장치 구조 및 연결 현황").heading());
                ui.label("SSEP 데이지 체인 없이 단일 모듈에 직접 접속하여 초고속 독립 계측(1 ~ 5,000 Hz)을 수행합니다.");
                ui.separator();

                ui.group(|ui| {
                    ui.label(RichText::new("스탠드얼론 직접 통신 구조도").strong().color(Color32::from_rgb(0, 229, 255)));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(" [Host PC / Studio] ").background_color(Color32::from_rgb(30, 40, 60)).color(Color32::WHITE));
                        ui.label(format!(" ==(TCP Port {}: Telnet 명령 / Port {}: 바이너리 데이터)==> ", self.cmd_port, self.data_port));
                        ui.label(RichText::new(format!(" [DSA5000 단독 장치: S/N {} (16 채널)] ", self.system_info.serial_number))
                            .background_color(Color32::from_rgb(20, 70, 40))
                            .color(Color32::from_rgb(0, 230, 118))
                            .strong());
                    });
                });

                ui.add_space(8.0);

                ui.group(|ui| {
                    ui.label(RichText::new("단독 계측 장비 상세 정보").strong().color(Color32::from_rgb(0, 229, 255)));
                    egui::Grid::new("standalone_info_grid")
                        .striped(true)
                        .min_col_width(120.0)
                        .spacing([20.0, 8.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("장비 모델:").strong());
                            ui.label(&self.system_info.model);
                            ui.end_row();

                            ui.label(RichText::new("시리얼 번호:").strong());
                            ui.label(&self.system_info.serial_number);
                            ui.end_row();

                            ui.label(RichText::new("접속 IP / 포트:").strong());
                            ui.label(format!("{}:{} (Cmd) / {} (Data)", self.host, self.cmd_port, self.data_port));
                            ui.end_row();

                            ui.label(RichText::new("동작 채널 수:").strong());
                            ui.label("16 채널 (Ch 01 ~ Ch 16 Standalone)");
                            ui.end_row();

                            ui.label(RichText::new("계측 속도:").strong());
                            ui.label(format!("{} Hz (Scan Rate: 최대 5,000 samples/sec/ch)", self.scan_rate_hz));
                            ui.end_row();

                            ui.label(RichText::new("공압 밸브 상태:").strong());
                            ui.label(&self.system_info.valve_mode);
                            ui.end_row();

                            ui.label(RichText::new("공학 단위:").strong());
                            ui.label(&self.system_info.engineering_units);
                            ui.end_row();
                        });
                });
            } else {
                ui.label(RichText::new(format!("DSA5000 스탠드얼론 다중 장비 동시 계측 현황 (총 {}대 / {} 채널)", mod_count, mod_count * 16)).heading());
                ui.label("여러 대의 DSA5000 물리 장비(또는 시뮬레이터)를 SSEP 동기 케이블 없이 각각 독립 IP로 직접 접속하여, 총 16xN 채널을 병렬로 동시 계측합니다.");
                ui.separator();

                ui.group(|ui| {
                    ui.label(RichText::new("스탠드얼론 다중 장비 병렬 통신 구조도").strong().color(Color32::from_rgb(0, 229, 255)));
                    ui.vertical(|ui| {
                        for (i, dev) in self.standalone_devices.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(" [Host PC / Studio] ").background_color(Color32::from_rgb(30, 40, 60)).color(Color32::WHITE));
                                ui.label(format!(" ===(TCP {}:{} / Data {})===> ", dev.host, dev.cmd_port, dev.data_port));
                                let ch_start = i * 16 + 1;
                                let ch_end = (i + 1) * 16;
                                ui.label(RichText::new(format!(" [모듈 {} ({}) : Ch {:02}~{:02} (16채널)] ", i, dev.label, ch_start, ch_end))
                                    .background_color(Color32::from_rgb(20, 60, 45))
                                    .color(Color32::from_rgb(0, 230, 118))
                                    .strong());
                            });
                        }
                    });
                });

                ui.add_space(8.0);

                ui.group(|ui| {
                    ui.label(RichText::new("🌐 전체 장비 동시 제어 (Broadcast)").strong().color(Color32::from_rgb(0, 229, 255)));
                    ui.horizontal(|ui| {
                        ui.label("밸브 동시 전환:");
                        if ui.button("전체 Px (계측)").clicked() {
                            self.handle_set_valve_target(None, ValveMode::Px);
                        }
                        if ui.button("전체 Cal (교정)").clicked() {
                            self.handle_set_valve_target(None, ValveMode::Cal);
                        }
                        if ui.button("전체 Prg (퍼지)").clicked() {
                            self.handle_set_valve_target(None, ValveMode::Prg);
                        }
                        if ui.button("전체 Iso (차단)").clicked() {
                            self.handle_set_valve_target(None, ValveMode::Iso);
                        }
                        ui.separator();
                        if ui.add_enabled(self.is_connected && !self.is_calibrating, egui::Button::new(RichText::new("⚡ 전체 동시 CALZ").color(Color32::from_rgb(255, 234, 0)))).clicked() {
                            self.handle_calz_target(None);
                        }
                        if ui.add_enabled(self.is_connected && !self.is_calibrating, egui::Button::new(RichText::new("⚡ 전체 즉각 Tare").color(Color32::from_rgb(0, 230, 118)))).clicked() {
                            self.handle_zero_tare(None);
                        }
                    });
                });

                ui.add_space(8.0);

                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("등록된 스탠드얼론 장비 목록 및 개별 제어").strong().color(Color32::from_rgb(0, 229, 255)));
                        if ui.button(RichText::new("⚙ 장치 관리자 열기 (IP 추가/삭제)").color(Color32::from_rgb(255, 234, 0))).clicked() {
                            self.show_device_manager_modal = true;
                        }
                    });
                    ui.add_space(4.0);
                    egui::Grid::new("standalone_multi_info_grid")
                        .striped(true)
                        .min_col_width(70.0)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("모듈").strong());
                            ui.label(RichText::new("장치 라벨").strong());
                            ui.label(RichText::new("IP 및 포트").strong());
                            ui.label(RichText::new("할당 채널").strong());
                            ui.label(RichText::new("연결").strong());
                            ui.label(RichText::new("현재 밸브").strong());
                            ui.label(RichText::new("개별 밸브 제어").strong());
                            ui.label(RichText::new("개별 교정").strong());
                            ui.end_row();

                            let dev_list = self.standalone_devices.clone();
                            let mut action_valve: Option<(usize, ValveMode)> = None;
                            let mut action_calz: Option<usize> = None;
                            let mut action_tare: Option<usize> = None;

                            for (i, dev) in dev_list.iter().enumerate() {
                                ui.label(format!("모듈 {}", i));
                                ui.label(&dev.label);
                                ui.label(format!("{}:{}/{}", dev.host, dev.cmd_port, dev.data_port));
                                ui.label(format!("Ch {:02} ~ Ch {:02}", i * 16 + 1, (i + 1) * 16));
                                if self.is_connected {
                                    ui.label(RichText::new("● 정상").color(Color32::from_rgb(0, 230, 118)));
                                } else {
                                    ui.label(RichText::new("○ 대기").color(Color32::GRAY));
                                }

                                let v_curr = self.module_valves.get(i).cloned().unwrap_or_else(|| "PX".into());
                                ui.label(RichText::new(&v_curr).strong().color(Color32::from_rgb(0, 229, 255)));

                                ui.horizontal(|ui| {
                                    if ui.selectable_label(v_curr == "PX", "Px").clicked() {
                                        action_valve = Some((i, ValveMode::Px));
                                    }
                                    if ui.selectable_label(v_curr == "CAL", "Cal").clicked() {
                                        action_valve = Some((i, ValveMode::Cal));
                                    }
                                    if ui.selectable_label(v_curr == "PRG", "Prg").clicked() {
                                        action_valve = Some((i, ValveMode::Prg));
                                    }
                                    if ui.selectable_label(v_curr == "ISO", "Iso").clicked() {
                                        action_valve = Some((i, ValveMode::Iso));
                                    }
                                });

                                ui.horizontal(|ui| {
                                    if ui.add_enabled(self.is_connected && !self.is_calibrating, egui::Button::new("⚡ CALZ")).clicked() {
                                        action_calz = Some(i);
                                    }
                                    if ui.add_enabled(self.is_connected && !self.is_calibrating, egui::Button::new("⚡ Tare")).on_hover_text("이 모듈 16채널을 즉시 0.000 PSI로 설정").clicked() {
                                        action_tare = Some(i);
                                    }
                                });
                                ui.end_row();
                            }

                            if let Some((idx, mode)) = action_valve {
                                self.handle_set_valve_target(Some(idx), mode);
                            }
                            if let Some(idx) = action_calz {
                                self.handle_calz_target(Some(idx));
                            }
                            if let Some(idx) = action_tare {
                                self.handle_zero_tare(Some(idx));
                            }
                        });
                });
            }

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button(RichText::new("⚙ 장치 목록 / IP 추가 및 삭제").color(Color32::from_rgb(0, 229, 255))).clicked() {
                    self.show_device_manager_modal = true;
                }
                ui.label(RichText::new("💡 PTP 하드웨어 동기화 데이지 체인이 필요한 경우:").color(Color32::from_rgb(255, 234, 0)));
                if ui.button(RichText::new("🔗 SSEP 다중 모듈 동기화 모드로 전환 (SSEP 0)").strong().color(Color32::from_rgb(0, 229, 255))).clicked() {
                    self.switch_operating_mode(OperatingMode::Ssep);
                }
            });

            return;
        }

        ui.label(RichText::new("SSEP (Scanivalve Synchronous Ethernet Protocol) 네트워크 토폴로지").heading());
        ui.label("DSA5000 매뉴얼 Section 3.2 및 Section 5.3 데이지 체인 라인 토폴로지 동기화 현황");
        ui.horizontal(|ui| {
            ui.label(RichText::new("💡 개별 직접 접속 초고속 계측이 필요한 경우:").color(Color32::from_rgb(255, 234, 0)));
            if ui.button(RichText::new("📦 스탠드얼론 모드로 전환 (SSEP 1)").strong().color(Color32::from_rgb(0, 229, 255))).clicked() {
                self.switch_operating_mode(OperatingMode::Standalone);
            }
        });
        ui.separator();

        // Topology Diagram
        ui.group(|ui| {
            ui.label(RichText::new("물리적 SSEP 라인 토폴로지 구조도").strong().color(Color32::from_rgb(0, 229, 255)));
            ui.horizontal(|ui| {
                ui.label(RichText::new(" [Host PC] ").background_color(Color32::from_rgb(30, 40, 60)).color(Color32::WHITE));
                ui.label(" ==(Gigabit Eth)==> ");

                let mods = if self.ssep_state.modules.is_empty() {
                    vec![crate::protocol::SsepModuleInfo {
                        index: 0,
                        model: "DSA5000".into(),
                        serial_number: 50421,
                        npr: "15.00".into(),
                        version: "2.01".into(),
                        rate_hz: 100.0,
                        valve_mode: "PX".into(),
                        units: "PSI".into(),
                        ip: self.host.clone(),
                        label: "CONTROLLER".into(),
                    }]
                } else {
                    self.ssep_state.modules.clone()
                };

                for (idx, m) in mods.iter().enumerate() {
                    let role_tag = if idx == 0 { "CONTROLLER" } else { "RESPONDER" };
                    let bg = if idx == 0 { Color32::from_rgb(0, 80, 120) } else { Color32::from_rgb(40, 70, 50) };

                    ui.label(
                        RichText::new(format!(" [{}: #{} (SN:{})] ", role_tag, m.index, m.serial_number))
                            .background_color(bg)
                            .color(Color32::WHITE)
                            .strong(),
                    );

                    if idx + 1 < mods.len() {
                        ui.label(" ==(M8 SSEP)==> ");
                    }
                }
            });
        });

        ui.add_space(8.0);

        // Explicit SSEP Role Assignment Panel (Scanivalve Manual V2.01 P.79 & P.102)
        ui.group(|ui| {
            ui.label(RichText::new("⚙ SSEP 역할 명시적 설정 (Controller & Responder 수동/자동 지정)").strong().color(Color32::from_rgb(0, 229, 255)));
            ui.label("Scanivalve 매뉴얼 규격: 0=자동 할당(Auto), 2=Controller 강제, 3=Responder(중간) 강제, 4=Last Responder(끝단) 강제, 1=Standalone");
            ui.add_space(4.0);

            // Per-device explicit table if multiple devices configured
            if self.standalone_devices.len() > 1 {
                ui.label(RichText::new("📋 연결 장치별 SSEP 역할 명시적 지정 목록").strong());
                let mut changed = false;
                let mut apply_target = None;
                let mut query_target = None;
                let num_devs = self.standalone_devices.len();
                egui::Grid::new("ssep_topo_explicit_grid")
                    .striped(true)
                    .min_col_width(70.0)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("모듈/채널").strong());
                        ui.label(RichText::new("IP 주소").strong());
                        ui.label(RichText::new("장치 라벨").strong());
                        ui.label(RichText::new("SSEP 역할 명시적 지정 (Role)").strong().color(Color32::from_rgb(0, 229, 255)));
                        ui.label(RichText::new("하드웨어 전송").strong());
                        ui.end_row();

                        for i in 0..num_devs {
                            let dev = &mut self.standalone_devices[i];
                            let ch_range = format!("Ch {:02}..{:02}", i * 16 + 1, (i + 1) * 16);
                            ui.colored_label(Color32::from_rgb(0, 229, 255), format!("Mod {} ({})", i, ch_range));
                            ui.label(&dev.host);
                            ui.label(&dev.label);

                            egui::ComboBox::from_id_salt(format!("ssep_topo_dev_role_{}", i))
                                .selected_text(dev.ssep_role.label())
                                .show_ui(ui, |ui| {
                                    for r in [
                                        crate::protocol::SsepAssignment::Controller,
                                        crate::protocol::SsepAssignment::MiddleResponder,
                                        crate::protocol::SsepAssignment::LastResponder,
                                        crate::protocol::SsepAssignment::Auto,
                                        crate::protocol::SsepAssignment::Standalone,
                                    ] {
                                        if ui.selectable_value(&mut dev.ssep_role, r, r.label()).clicked() {
                                            changed = true;
                                        }
                                    }
                                });

                            let role_val = dev.ssep_role;
                            ui.horizontal(|ui| {
                                if ui.button(RichText::new("✔ 적용 & SAVE").strong().color(Color32::from_rgb(0, 230, 118)))
                                    .on_hover_text(format!("모듈 {} ({})에 {} 적용 및 EEPROM 영구 저장", i, dev.host, role_val.label()))
                                    .clicked()
                                {
                                    apply_target = Some((i, role_val));
                                }
                                if ui.small_button("🔍 조회").clicked() {
                                    query_target = Some(i);
                                }
                            });
                            ui.end_row();
                        }
                    });

                if changed {
                    self.save_config();
                }

                if let Some((idx, r)) = apply_target {
                    self.handle_apply_and_save_ssep_role(idx, r);
                }
                if let Some(idx) = query_target {
                    self.handle_query_ssep(Some(idx));
                }

                ui.add_space(4.0);
            }

            ui.horizontal(|ui| {
                ui.label("제어 대상:");
                let target_text = match self.control_target {
                    None => "전체 모듈 (Broadcast)".to_string(),
                    Some(idx) => format!("모듈 {} (Index {})", idx, idx),
                };
                egui::ComboBox::from_id_salt("ssep_role_target_combo")
                    .selected_text(target_text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.control_target, None, "전체 모듈 (Broadcast)");
                        let total_m = self.current_module_count();
                        for i in 0..total_m {
                            let mod_name = if i == 0 {
                                format!("모듈 0 (Controller 후보)")
                            } else {
                                format!("모듈 {} (Responder 후보)", i)
                            };
                            ui.selectable_value(&mut self.control_target, Some(i), mod_name);
                        }
                    });

                ui.separator();

                if ui.button(RichText::new("👑 Controller (SSEP 2)").strong().color(Color32::from_rgb(0, 229, 255)))
                    .on_hover_text("대상 모듈을 SSEP 마스터 Controller로 강제 지정합니다.")
                    .clicked()
                {
                    self.handle_set_ssep_assignment(self.control_target, 2);
                }

                if ui.button(RichText::new("🔗 Responder (SSEP 3)").strong().color(Color32::from_rgb(0, 230, 118)))
                    .on_hover_text("대상 모듈을 SSEP 중간 Responder로 강제 지정합니다.")
                    .clicked()
                {
                    self.handle_set_ssep_assignment(self.control_target, 3);
                }

                if ui.button(RichText::new("🏁 Last Responder (SSEP 4)").strong().color(Color32::from_rgb(255, 170, 0)))
                    .on_hover_text("대상 모듈을 체인의 마지막 Last Responder로 강제 지정합니다.")
                    .clicked()
                {
                    self.handle_set_ssep_assignment(self.control_target, 4);
                }

                if ui.button(RichText::new("⚡ Auto (SSEP 0)").color(Color32::LIGHT_GRAY))
                    .on_hover_text("하드웨어 포트 연결 상태에 따른 자동 할당(기본값 0)으로 설정합니다.")
                    .clicked()
                {
                    self.handle_set_ssep_assignment(self.control_target, 0);
                }

                if ui.button(RichText::new("📦 Standalone (SSEP 1)").color(Color32::from_rgb(255, 100, 100)))
                    .on_hover_text("SSEP 비활성화 및 스탠드얼론 독립 모드로 강제 전환합니다.")
                    .clicked()
                {
                    self.handle_set_ssep_assignment(self.control_target, 1);
                }
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button(RichText::new("🚀 지정된 SSEP 역할 일괄 적용 & Flash SAVE").strong().color(Color32::from_rgb(0, 229, 255)))
                    .on_hover_text("각 장치별로 설정된 Controller/Responder 역할을 장비에 전송하고 Flash EEPROM에 영구 저장합니다.")
                    .clicked()
                {
                    self.handle_apply_designated_ssep_roles();
                }

                if ui.button(RichText::new("⚡ 기본 순환 배치 (Mod 0=Controller, 마지막=Last Responder) & SAVE").color(Color32::LIGHT_GRAY))
                    .on_hover_text("모듈 0번은 SSEP 2(Controller), 마지막 모듈은 SSEP 4(Last Responder)를 전송하고 SAVE를 수행합니다.")
                    .clicked()
                {
                    self.handle_configure_ssep_chain();
                }

                if ui.button(RichText::new("💾 하드웨어 Flash 영구 저장 (SAVE)").strong().color(Color32::from_rgb(0, 230, 118)))
                    .on_hover_text("설정한 SSEP 역할을 모듈 EEPROM에 영구 저장합니다. 전원 재부팅 후에도 유지됩니다.")
                    .clicked()
                {
                    self.handle_save_hardware(self.control_target);
                }

                if ui.button("🔍 현재 SSEP 조회").clicked() {
                    self.handle_query_ssep(self.control_target);
                }

                if ui.button("📡 SMODE 조회").clicked() {
                    self.terminal_input = "SMODE".into();
                    self.handle_send_terminal();
                }

                if ui.button("📋 LIST SYS * 새로고침").clicked() {
                    self.terminal_input = "LIST SYS *".into();
                    self.handle_send_terminal();
                }
            });
        });

        ui.add_space(8.0);

        // Module Table (LIST SYS *)
        ui.label(RichText::new("SSEP 장치 목록 (`LIST SYS *` 동기화)").strong());
        egui::ScrollArea::vertical().id_salt("ssep_table_scroll").show(ui, |ui| {
            egui::Grid::new("ssep_modules_table")
                .striped(true)
                .min_col_width(70.0)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Index").strong());
                    ui.label(RichText::new("역할 (Role)").strong());
                    ui.label(RichText::new("모델 (Model)").strong());
                    ui.label(RichText::new("S/N").strong());
                    ui.label(RichText::new("압력 범위 (NPR)").strong());
                    ui.label(RichText::new("Firmware").strong());
                    ui.label(RichText::new("Rate (Hz)").strong());
                    ui.label(RichText::new("밸브 상태").strong());
                    ui.label(RichText::new("IP 주소").strong());
                    ui.label(RichText::new("라벨").strong());
                    ui.end_row();

                    for m in &self.ssep_state.modules {
                        let is_ctrl = m.index == 0;
                        let role = if is_ctrl { "Controller" } else { "Responder" };
                        let role_color = if is_ctrl { Color32::from_rgb(0, 229, 255) } else { Color32::from_rgb(0, 230, 118) };

                        ui.label(format!("{}", m.index));
                        ui.colored_label(role_color, role);
                        ui.label(&m.model);
                        ui.label(format!("{}", m.serial_number));
                        ui.label(&m.npr);
                        ui.label(&m.version);
                        ui.label(format!("{:.1}", m.rate_hz));
                        ui.label(&m.valve_mode);
                        ui.label(&m.ip);
                        ui.label(&m.label);
                        ui.end_row();
                    }
                });
        });

        ui.separator();

        // Diagnostics Quick Commands
        ui.horizontal(|ui| {
            ui.label(RichText::new("SSEP 진단 명령:").strong());
            if ui.button("SMODE (동기화 모드 확인)").clicked() {
                self.terminal_input = "SMODE".into();
                self.handle_send_terminal();
            }
            if ui.button("LIST SYS * (전체 상세)").clicked() {
                self.terminal_input = "LIST SYS *".into();
                self.handle_send_terminal();
            }
            if ui.button("SSEP 0 (자동 할당)").clicked() {
                self.terminal_input = "SSEP 0".into();
                self.handle_send_terminal();
            }
            if ui.button("SSEP 2 (강제 Controller)").clicked() {
                self.terminal_input = "SSEP 2".into();
                self.handle_send_terminal();
            }
            if ui.button("SSEP 1 (단독 Standalone)").clicked() {
                self.terminal_input = "SSEP 1".into();
                self.handle_send_terminal();
            }
        });
    }

    fn render_data_logger_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Scanivalve DSA5000 단일 통합 CSV 데이터 로거").heading());
        ui.label("연결된 모든 장비(단독 모듈 1~N대 / SSEP 전체 모듈)의 모든 압력 및 온도 채널이 하나의 단일 CSV 파일에 동시 기록됩니다.");
        ui.separator();

        ui.group(|ui| {
            ui.label(RichText::new("📁 CSV 저장 파일 설정 (Single Unified File)").strong().color(Color32::from_rgb(0, 229, 255)));
            ui.horizontal(|ui| {
                ui.label("저장 파일명:");
                if ui.add(egui::TextEdit::singleline(&mut self.log_base_name).desired_width(260.0).hint_text("예: DSA5000_Measurement.csv")).changed() {
                    self.save_config();
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut self.single_file_mode, "단일 파일 고정 저장 (Single File Mode - 분할 없음)").changed() {
                    self.save_config();
                }
                if self.single_file_mode {
                    if ui.checkbox(&mut self.append_to_existing, "기존 파일 끝에 이어쓰기 (Append)").changed() {
                        self.save_config();
                    }
                }
            });

            ui.add_space(4.0);
            let ts_example = crate::logger::CsvLogger::current_timestamp_str();
            let base_raw = self.log_base_name.trim();
            let stripped = base_raw.strip_suffix(".csv").unwrap_or(base_raw);
            let safe_stem = if stripped.is_empty() { "DSA5000_Measurement" } else { stripped };
            let display_name = if self.single_file_mode && self.append_to_existing {
                format!("{}.csv", safe_stem)
            } else {
                format!("{}_{}.csv", safe_stem, ts_example)
            };
            ui.label(RichText::new(format!("실제 저장 대상 (년월일시분초 포함): ./data_records/{}", display_name)).small().color(Color32::from_rgb(170, 255, 170)));
        });

        ui.add_space(8.0);

        let (btn_txt, btn_color) = if self.logger.is_recording {
            (
                format!("■ 기록 종료 (저장 완료: {} 프레임)", self.logger.recorded_frames),
                Color32::from_rgb(255, 234, 0),
            )
        } else {
            ("● CSV 기록 시작 (단일 파일 저장)".into(), Color32::from_rgb(255, 60, 60))
        };

        if ui.add_enabled(self.is_connected, egui::Button::new(RichText::new(btn_txt).strong().color(btn_color))).clicked() {
            self.toggle_logging();
        }

        ui.separator();

        if let Some(ref path) = self.last_logged_path {
            ui.horizontal(|ui| {
                ui.label(format!("최근 기록 파일: {}", path));
                if ui.button("📂 저장 폴더 열기 (Explorer)").clicked() {
                    let _ = std::process::Command::new("explorer").arg("./data_records").spawn();
                }
            });
        }
    }

    fn render_terminal_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("인터랙티브 Telnet 콘솔 터미널 (Port 23)").heading());
        ui.separator();

        let mod_cnt = self.current_module_count();
        if mod_cnt > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("명령 대상 장치:").strong().color(Color32::from_rgb(0, 229, 255)));
                ui.selectable_value(&mut self.terminal_target, None, "🌐 전체 동시 전송 (Broadcast)");
                for i in 0..mod_cnt {
                    ui.selectable_value(&mut self.terminal_target, Some(i), format!("모듈 {}", i));
                }
            });
            ui.separator();
        }

        // Quick Buttons
        ui.horizontal_wrapped(|ui| {
            ui.label("빠른 명령:");
            if ui.button("STATUS").clicked() {
                self.terminal_input = "STATUS".into();
                self.handle_send_terminal();
            }
            if ui.button("LIST SYS *").clicked() {
                self.terminal_input = "LIST SYS *".into();
                self.handle_send_terminal();
            }
            if ui.button("SMODE").clicked() {
                self.terminal_input = "SMODE".into();
                self.handle_send_terminal();
            }
            if ui.button("LIST S").clicked() {
                self.terminal_input = "LIST S".into();
                self.handle_send_terminal();
            }
            if ui.button("VALVE").clicked() {
                self.terminal_input = "VALVE".into();
                self.handle_send_terminal();
            }
            if ui.button("RATE").clicked() {
                self.terminal_input = "RATE".into();
                self.handle_send_terminal();
            }
            if ui.button("SSEP").clicked() {
                self.terminal_input = "SSEP".into();
                self.handle_send_terminal();
            }
            if ui.button("SSEP 0 (SSEP Auto)").clicked() {
                self.terminal_input = "SSEP 0".into();
                self.handle_send_terminal();
            }
            if ui.button("SSEP 1 (Standalone)").clicked() {
                self.terminal_input = "SSEP 1".into();
                self.handle_send_terminal();
            }
        });

        ui.separator();

        // Terminal Output Log Window
        let term_height = (ui.available_height() - 40.0).max(200.0);
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .max_height(term_height)
            .show(ui, |ui| {
                for (line, color) in &self.terminal_history {
                    ui.colored_label(*color, line);
                }
            });

        ui.separator();

        // Terminal Input Field
        ui.horizontal(|ui| {
            ui.label(RichText::new(">").strong().color(Color32::from_rgb(0, 229, 255)));
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.terminal_input)
                    .desired_width(ui.available_width() - 80.0)
                    .font(egui::TextStyle::Monospace),
            );

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.handle_send_terminal();
                response.request_focus();
            }

            if ui.button("전송 (Send)").clicked() {
                self.handle_send_terminal();
            }
        });
    }
}
