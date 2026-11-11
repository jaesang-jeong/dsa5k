// Scanivalve DSA5000 Binary Protocol & SSEP Definitions in Pure Rust
// Based on Scanivalve DSA5000 Operations & Service Manual (Software Version 2.01)

pub const PACKET_ID: u16 = 0x0200;
pub const GLOBAL_HEADER_SIZE_STD: usize = 28; // DSA5000 Manual Table 5.1 Standard (28 bytes)
pub const GLOBAL_HEADER_SIZE_ALT: usize = 36; // Legacy / Alternate simulator header size
pub const MODULE_DATA_SIZE: usize = 140;
pub const CHANNELS_PER_MODULE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValveMode {
    Px,   // Measurement Mode (Sensor connected to port Px)
    Cal,  // Calibration Mode (Sensor connected to CAL reference manifold)
    Prg,  // Purge Mode (Sensor connected to purge air blow-out)
    Iso,  // Isolate Mode (Sensor isolated from inputs)
    Off,  // Valve Solenoids De-energized
}

impl ValveMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValveMode::Px => "PX",
            ValveMode::Cal => "CAL",
            ValveMode::Prg => "PRG",
            ValveMode::Iso => "ISO",
            ValveMode::Off => "OFF",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ValveMode::Px => "MEASUREMENT (Px)",
            ValveMode::Cal => "CALIBRATION (Cal)",
            ValveMode::Prg => "PURGE (Prg)",
            ValveMode::Iso => "ISOLATE (Iso)",
            ValveMode::Off => "OFF (De-energized)",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_uppercase().as_str() {
            "PX" | "RUN" => Some(ValveMode::Px),
            "CAL" => Some(ValveMode::Cal),
            "PRG" | "PURGE" => Some(ValveMode::Prg),
            "ISO" => Some(ValveMode::Iso),
            "OFF" => Some(ValveMode::Off),
            _ => None,
        }
    }
}

/// SSEP Assignment Mode (Manual page 79 & 102)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SsepAssignment {
    Auto = 0,          // Automatic assignment based on physical SSEP In/Out port link status
    Standalone = 1,    // Forced Standalone mode (ignores SSEP ports)
    Controller = 2,    // Forced Controller mode
    MiddleResponder = 3, // Forced Middle Responder
    LastResponder = 4, // Forced Last Responder (terminates chain)
}

impl Default for SsepAssignment {
    fn default() -> Self {
        SsepAssignment::Auto
    }
}

impl SsepAssignment {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => SsepAssignment::Standalone,
            2 => SsepAssignment::Controller,
            3 => SsepAssignment::MiddleResponder,
            4 => SsepAssignment::LastResponder,
            _ => SsepAssignment::Auto,
        }
    }

    pub fn to_u8(&self) -> u8 {
        *self as u8
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SsepAssignment::Auto => "Auto (0)",
            SsepAssignment::Standalone => "Standalone (1)",
            SsepAssignment::Controller => "Controller (2)",
            SsepAssignment::MiddleResponder => "Middle Responder (3)",
            SsepAssignment::LastResponder => "Last Responder (4)",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SsepAssignment::Auto => "SSEP 0 (자동 할당 Auto)",
            SsepAssignment::Controller => "SSEP 2 (강제 Controller)",
            SsepAssignment::MiddleResponder => "SSEP 3 (강제 Responder)",
            SsepAssignment::LastResponder => "SSEP 4 (강제 Last Responder)",
            SsepAssignment::Standalone => "SSEP 1 (강제 Standalone)",
        }
    }
}

/// Discovered module information from SSEP `LIST SYS` or `LIST SYS *`
#[derive(Debug, Clone)]
pub struct SsepModuleInfo {
    pub index: usize,
    pub model: String,
    pub serial_number: u32,
    pub npr: String,
    pub version: String,
    pub rate_hz: f32,
    pub valve_mode: String,
    pub units: String,
    pub ip: String,
    pub label: String,
}

/// Overall SSEP System Status from `SMODE` and `LIST SYS *`
#[derive(Debug, Clone)]
pub struct SsepSystemState {
    pub unit_role: String,
    pub address: u8,
    pub num_responders: usize,
    pub link_in: String,
    pub link_out: String,
    pub modules: Vec<SsepModuleInfo>,
}

impl Default for SsepSystemState {
    fn default() -> Self {
        Self {
            unit_role: "Controller".into(),
            address: 0,
            num_responders: 0,
            link_in: "DOWN".into(),
            link_out: "DOWN".into(),
            modules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GlobalHeader {
    pub packet_id: u16,
    pub num_modules: u16,
    pub frame_number: u32,
    pub ptp_sec: u32,
    pub ptp_nsec: u32,
}

#[derive(Debug, Clone)]
pub struct ModuleData {
    pub is_dsa: bool,
    pub serial_number: u16,
    pub responder_addr: u8,
    pub is_scan_packet: bool,
    pub pressures: [f32; CHANNELS_PER_MODULE],
    pub temperatures: [f32; CHANNELS_PER_MODULE],
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub header: GlobalHeader,
    pub modules: Vec<ModuleData>,
    pub timestamp_ms: u64,
}

impl Packet {
    pub fn total_channels(&self) -> usize {
        self.modules.len() * CHANNELS_PER_MODULE
    }

    /// Retrieve flat channel pressure across all SSEP modules (0..total_channels)
    pub fn get_channel_pressure(&self, global_ch: usize) -> Option<f32> {
        let mod_idx = global_ch / CHANNELS_PER_MODULE;
        let ch_idx = global_ch % CHANNELS_PER_MODULE;
        self.modules.get(mod_idx).map(|m| m.pressures[ch_idx])
    }

    /// Retrieve flat channel temperature across all SSEP modules (0..total_channels)
    pub fn get_channel_temperature(&self, global_ch: usize) -> Option<f32> {
        let mod_idx = global_ch / CHANNELS_PER_MODULE;
        let ch_idx = global_ch % CHANNELS_PER_MODULE;
        self.modules.get(mod_idx).map(|m| m.temperatures[ch_idx])
    }
}

/// Parse a raw binary buffer into a Packet
/// Supports both Little-Endian (Real DSA5000 v2.01 hardware) and Big-Endian (Network byte order / Simulator)
/// Returns Some((Packet, bytes_consumed)) or None if incomplete/need more bytes
pub fn parse_binary_packet(buffer: &[u8]) -> Option<(Packet, usize)> {
    if buffer.len() < GLOBAL_HEADER_SIZE_ALT {
        return None;
    }

    // Find magic 0x0200 in either Little Endian [0x00, 0x02] or Big Endian [0x02, 0x00]
    let mut sync_offset = None;
    let mut is_le = false;

    if buffer.len() >= 4 {
        for i in 0..=(buffer.len() - 4) {
            // Check Little Endian: [0x00, 0x02, num_modules_le, 0x00]
            if buffer[i] == 0x00 && buffer[i + 1] == 0x02 {
                let n_mods = u16::from_le_bytes([buffer[i + 2], buffer[i + 3]]);
                if n_mods >= 1 && n_mods <= 16 {
                    sync_offset = Some(i);
                    is_le = true;
                    break;
                }
            }
            // Check Big Endian: [0x02, 0x00, 0x00, num_modules_be]
            if buffer[i] == 0x02 && buffer[i + 1] == 0x00 {
                let n_mods = u16::from_be_bytes([buffer[i + 2], buffer[i + 3]]);
                if n_mods >= 1 && n_mods <= 16 {
                    sync_offset = Some(i);
                    is_le = false;
                    break;
                }
            }
        }
    }

    let offset = match sync_offset {
        Some(pos) => pos,
        None => return None,
    };

    let slice = &buffer[offset..];
    if slice.len() < GLOBAL_HEADER_SIZE_ALT {
        return None;
    }

    let packet_id = if is_le {
        u16::from_le_bytes([slice[0], slice[1]])
    } else {
        u16::from_be_bytes([slice[0], slice[1]])
    };

    let num_modules = if is_le {
        u16::from_le_bytes([slice[2], slice[3]])
    } else {
        u16::from_be_bytes([slice[2], slice[3]])
    };

    if num_modules == 0 || num_modules > 16 {
        return None; // Invalid module count
    }

    // Header size determination:
    // Physical DSA5000 hardware (Little-Endian) strictly adheres to Manual Table 5.1 (28 bytes).
    // For Big-Endian (Simulator / Network order): check whether module header starts at offset 28 or legacy 36.
    let header_size = if is_le {
        GLOBAL_HEADER_SIZE_STD // 28 bytes
    } else if slice.len() >= 32 && ((slice[28] & 0x80) != 0 || slice[31] == 0x02) {
        GLOBAL_HEADER_SIZE_STD // 28 bytes
    } else if slice.len() >= 40 && ((slice[36] & 0x80) != 0 || slice[39] == 0x02) {
        GLOBAL_HEADER_SIZE_ALT // 36 bytes (legacy simulator)
    } else {
        GLOBAL_HEADER_SIZE_STD // Standard 28 bytes
    };

    let expected_len = header_size + (num_modules as usize * MODULE_DATA_SIZE);
    if slice.len() < expected_len {
        return None;
    }

    let frame_number = if is_le {
        u32::from_le_bytes([slice[4], slice[5], slice[6], slice[7]])
    } else {
        u32::from_be_bytes([slice[4], slice[5], slice[6], slice[7]])
    };

    let ptp_sec = if is_le {
        u32::from_le_bytes([slice[8], slice[9], slice[10], slice[11]])
    } else {
        u32::from_be_bytes([slice[8], slice[9], slice[10], slice[11]])
    };

    let ptp_nsec = if is_le {
        u32::from_le_bytes([slice[12], slice[13], slice[14], slice[15]])
    } else {
        u32::from_be_bytes([slice[12], slice[13], slice[14], slice[15]])
    };

    let mut modules = Vec::with_capacity(num_modules as usize);
    let mut mod_offset = header_size;

    for _ in 0..num_modules {
        let mod_type_sn = if is_le {
            u16::from_le_bytes([slice[mod_offset], slice[mod_offset + 1]])
        } else {
            u16::from_be_bytes([slice[mod_offset], slice[mod_offset + 1]])
        };
        let is_dsa = (mod_type_sn & 0x8000) != 0;
        let serial_number = mod_type_sn & 0x7FFF;
        let responder_addr = slice[mod_offset + 2];
        let status_byte = slice[mod_offset + 3];
        let is_scan_packet = (status_byte & 0x02) != 0;

        let press_offset = mod_offset + 12;
        let mut pressures = [0.0f32; CHANNELS_PER_MODULE];
        for ch in 0..CHANNELS_PER_MODULE {
            let p_off = press_offset + (ch * 4);
            let b = [slice[p_off], slice[p_off + 1], slice[p_off + 2], slice[p_off + 3]];
            pressures[ch] = if is_le { f32::from_le_bytes(b) } else { f32::from_be_bytes(b) };
        }

        let temp_offset = press_offset + (CHANNELS_PER_MODULE * 4);
        let mut temperatures = [0.0f32; CHANNELS_PER_MODULE];
        for ch in 0..CHANNELS_PER_MODULE {
            let t_off = temp_offset + (ch * 4);
            let b = [slice[t_off], slice[t_off + 1], slice[t_off + 2], slice[t_off + 3]];
            temperatures[ch] = if is_le { f32::from_le_bytes(b) } else { f32::from_be_bytes(b) };
        }

        modules.push(ModuleData {
            is_dsa,
            serial_number,
            responder_addr,
            is_scan_packet,
            pressures,
            temperatures,
        });

        mod_offset += MODULE_DATA_SIZE;
    }

    let timestamp_ms = (ptp_sec as u64 * 1000) + (ptp_nsec as u64 / 1_000_000);

    Some((
        Packet {
            header: GlobalHeader {
                packet_id,
                num_modules,
                frame_number,
                ptp_sec,
                ptp_nsec,
            },
            modules,
            timestamp_ms,
        },
        offset + expected_len,
    ))
}
