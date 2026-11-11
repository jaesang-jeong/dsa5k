// High-Speed Streaming CSV Logger for DSA5000 in Pure Rust
// Full Multi-Module SSEP Support

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::SystemInfo;
use crate::protocol::{Packet, CHANNELS_PER_MODULE};

pub struct CsvLogger {
    output_dir: PathBuf,
    writer: Option<BufWriter<File>>,
    current_file_path: Option<PathBuf>,
    pub recorded_frames: u64,
    pub recorded_bytes: u64,
    pub is_recording: bool,
    header_written: bool,
    units: String,
    pub channel_names: Vec<String>,
    pub single_file_mode: bool,
    pub append_to_existing: bool,
    first_ptp_time: Option<f64>,
    start_instant: Option<std::time::Instant>,
}

impl CsvLogger {
    pub fn new<P: AsRef<Path>>(output_dir: P) -> Self {
        let path = output_dir.as_ref().to_path_buf();
        let _ = fs::create_dir_all(&path);
        Self {
            output_dir: path,
            writer: None,
            current_file_path: None,
            recorded_frames: 0,
            recorded_bytes: 0,
            is_recording: false,
            header_written: false,
            units: "PSI".into(),
            channel_names: Vec::new(),
            single_file_mode: true,
            append_to_existing: false,
            first_ptp_time: None,
            start_instant: None,
        }
    }

    pub fn set_channel_names(&mut self, names: Vec<String>) {
        self.channel_names = names;
    }

    /// Generates local timestamp string in YYYYMMDD_HHMMSS format without external crate dependencies
    pub fn current_timestamp_str() -> String {
        #[cfg(windows)]
        unsafe {
            #[repr(C)]
            #[allow(non_snake_case)]
            struct SYSTEMTIME {
                wYear: u16,
                wMonth: u16,
                wDayOfWeek: u16,
                wDay: u16,
                wHour: u16,
                wMinute: u16,
                wSecond: u16,
                wMilliseconds: u16,
            }
            extern "system" {
                fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
            }
            let mut st: SYSTEMTIME = std::mem::zeroed();
            GetLocalTime(&mut st);
            format!(
                "{:04}{:02}{:02}_{:02}{:02}{:02}",
                st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
            )
        }
        #[cfg(not(windows))]
        {
            let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            let days = secs / 86400;
            let day_secs = secs % 86400;
            let hour = day_secs / 3600;
            let minute = (day_secs % 3600) / 60;
            let second = day_secs % 60;
            format!("UTC_{}_{:02}{:02}{:02}", days, hour, minute, second)
        }
    }

    pub fn start(&mut self, info: &SystemInfo, base_name: &str) -> io::Result<PathBuf> {
        if self.is_recording {
            self.stop()?;
        }

        let ts = Self::current_timestamp_str();
        let file_path = if self.single_file_mode {
            let mut name = base_name.trim().to_string();
            if name.is_empty() {
                name = "DSA5000_Measurement".to_string();
            }
            let stripped = name.strip_suffix(".csv").unwrap_or(&name);
            let final_name = if self.append_to_existing {
                // If appending to existing fixed name, keep user's explicit filename
                if !name.to_lowercase().ends_with(".csv") {
                    format!("{}.csv", name)
                } else {
                    name
                }
            } else {
                format!("{}_{}.csv", stripped, ts)
            };
            self.output_dir.join(&final_name)
        } else {
            let mut safe_name = base_name.trim();
            if safe_name.is_empty() {
                safe_name = "DSA5000_Scan";
            }
            let stripped = safe_name.strip_suffix(".csv").unwrap_or(safe_name);
            let file_name = format!("{}_{}.csv", stripped, ts);
            self.output_dir.join(&file_name)
        };

        let file_exists = file_path.exists();
        let should_append = self.single_file_mode && self.append_to_existing && file_exists;

        let mut final_path = file_path.clone();
        let mut final_should_append = should_append;

        let open_res = if final_should_append {
            fs::OpenOptions::new().create(true).append(true).open(&final_path)
        } else {
            File::create(&final_path)
        };

        let file = match open_res {
            Ok(f) => f,
            Err(orig_err) => {
                // If file is locked by Excel or another process, auto-generate alternate indexed name
                let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("DSA5000_Scan");
                let mut fallback_file = None;
                for idx in 1..=50 {
                    let alt_name = format!("{}_{}.csv", stem, idx);
                    let alt_path = self.output_dir.join(&alt_name);
                    if let Ok(f) = File::create(&alt_path) {
                        final_path = alt_path;
                        final_should_append = false;
                        fallback_file = Some(f);
                        break;
                    }
                }
                match fallback_file {
                    Some(f) => f,
                    None => return Err(orig_err),
                }
            }
        };

        let mut writer = BufWriter::with_capacity(128 * 1024, file); // 128 KB buffer

        if !final_should_append {
            // Write Scanivalve Metadata Header for single unified file
            writeln!(writer, "# Scanivalve DSA5000 Unified Multi-Device Measurement Log")?;
            writeln!(writer, "# Model: {}", info.model)?;
            writeln!(writer, "# Primary Serial Number: {}", info.serial_number)?;
            writeln!(writer, "# Scan Rate: {} Hz", info.scan_rate_hz)?;
            writeln!(writer, "# Units: {}", info.engineering_units)?;
            writeln!(writer, "# Valve Mode: {}", info.valve_mode)?;
            self.header_written = false;
        } else {
            self.header_written = true;
        }

        self.units = info.engineering_units.clone();
        self.writer = Some(writer);
        self.current_file_path = Some(final_path.clone());
        self.recorded_frames = 0;
        self.recorded_bytes = 0;
        self.first_ptp_time = None;
        self.start_instant = Some(std::time::Instant::now());
        self.is_recording = true;

        Ok(final_path)
    }

    pub fn write_frame(&mut self, packet: &Packet) -> io::Result<()> {
        let writer = match self.writer.as_mut() {
            Some(w) => w,
            None => return Ok(()),
        };

        // Write dynamic CSV columns on first frame (based on actual number of modules in SSEP packet)
        if !self.header_written {
            write!(writer, "Frame,Relative_Time_sec,PTP_Sec,PTP_Nsec,Timestamp_ms")?;
            for (m_idx, m) in packet.modules.iter().enumerate() {
                let role = if m_idx == 0 { "Ctrl" } else { "Resp" };
                for ch in 1..=CHANNELS_PER_MODULE {
                    let global_ch = (m_idx * CHANNELS_PER_MODULE) + ch;
                    let custom = self.channel_names.get(global_ch - 1).map(|s| s.trim()).filter(|s| !s.is_empty());
                    let name_tag = if let Some(n) = custom {
                        let safe: String = n.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
                        format!("_{}", safe)
                    } else {
                        String::new()
                    };
                    write!(
                        writer,
                        ",Ch{:02}{}_M{}_{}_SN{}_{}",
                        global_ch, name_tag, m_idx, role, m.serial_number, self.units
                    )?;
                }
            }
            for (m_idx, m) in packet.modules.iter().enumerate() {
                for ch in 1..=CHANNELS_PER_MODULE {
                    let global_ch = (m_idx * CHANNELS_PER_MODULE) + ch;
                    write!(writer, ",Temp_Ch{:02}_M{}_SN{}_C", global_ch, m_idx, m.serial_number)?;
                }
            }
            writeln!(writer)?;
            self.header_written = true;
        }

        let rel_time_sec = if packet.header.ptp_sec > 0 {
            let ptp_t = packet.header.ptp_sec as f64 + (packet.header.ptp_nsec as f64 / 1_000_000_000.0);
            if self.first_ptp_time.is_none() {
                self.first_ptp_time = Some(ptp_t);
            }
            ptp_t - self.first_ptp_time.unwrap()
        } else {
            self.start_instant.map(|i| i.elapsed().as_secs_f64()).unwrap_or(0.0)
        };

        write!(
            writer,
            "{},{:.6},{},{},{}",
            packet.header.frame_number,
            rel_time_sec,
            packet.header.ptp_sec,
            packet.header.ptp_nsec,
            packet.timestamp_ms
        )?;

        // Write all SSEP modules pressures
        for m in &packet.modules {
            for p in &m.pressures {
                write!(writer, ",{:.5}", p)?;
            }
        }

        // Write all SSEP modules temperatures
        for m in &packet.modules {
            for t in &m.temperatures {
                write!(writer, ",{:.2}", t)?;
            }
        }
        writeln!(writer)?;

        self.recorded_frames += 1;
        Ok(())
    }

    pub fn stop(&mut self) -> io::Result<Option<PathBuf>> {
        if !self.is_recording {
            return Ok(self.current_file_path.clone());
        }

        if let Some(mut w) = self.writer.take() {
            w.flush()?;
        }

        self.is_recording = false;
        let path = self.current_file_path.clone();

        if let Some(ref p) = path {
            if let Ok(metadata) = fs::metadata(p) {
                self.recorded_bytes = metadata.len();
            }
        }

        Ok(path)
    }
}
