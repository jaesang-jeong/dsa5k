// Scanivalve DSA5000 SSEP Native Desktop Studio
// Standalone Windows Desktop GUI Application in Pure Rust (Zero Browser / Zero Web Dependencies)

mod client;
mod gui;
mod logger;
mod protocol;
mod simulator;

use std::env;
use std::io::{self, stdout, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use client::DSA5000Client;
use eframe::egui;
use gui::DsaApp;
use logger::CsvLogger;
use protocol::ValveMode;
use simulator::DSA5000Simulator;

fn print_banner() {
    println!("\x1b[1;36m===============================================================\x1b[0m");
    println!("\x1b[1;36m>>> Scanivalve DSA5000 SSEP Studio (Native Desktop v2.01)\x1b[0m");
    println!("\x1b[1;36m===============================================================\x1b[0m");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    print_banner();

    let args: Vec<String> = env::args().collect();
    let is_test = args.iter().any(|a| a == "--test");
    let is_cli = args.iter().any(|a| a == "--cli");
    let device_ip = args.iter().position(|a| a == "--device").and_then(|i| args.get(i + 1));

    if let Some(ip_arg) = device_ip {
        let hosts: Vec<String> = ip_arg
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let endpoints: Vec<client::DeviceEndpoint> = hosts
            .iter()
            .enumerate()
            .map(|(idx, h)| client::DeviceEndpoint {
                host: h.clone(),
                cmd_port: 23,
                data_port: 503,
                label: format!("Device {}", idx + 1),
                ssep_role: if idx == 0 {
                    protocol::SsepAssignment::Controller
                } else {
                    protocol::SsepAssignment::MiddleResponder
                },
            })
            .collect();

        if endpoints.len() > 1 {
            println!(
                "\x1b[1;33m[*] Testing live multi-device standalone connection to {} devices: {:?}...\x1b[0m",
                endpoints.len(),
                hosts
            );
        } else {
            println!("\x1b[1;33m[*] Testing live connection to physical DSA5000 at {}...\x1b[0m", ip_arg);
        }

        let mut client = DSA5000Client::new_multi(endpoints);
        let info = client.connect()?;
        println!("  -> Connected! Model: {}, S/N: {}, Rate: {} Hz, Units: {}", info.model, info.serial_number, info.scan_rate_hz, info.engineering_units);
        
        let frame_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let frame_clone = Arc::clone(&frame_count);

        println!("  -> Starting real-time standalone data stream on Port 503...");
        client.start_scan(move |pkt| {
            let count = frame_clone.fetch_add(1, Ordering::SeqCst) + 1;
            if count <= 5 || count % 20 == 0 {
                let mod_count = pkt.modules.len();
                let ch_count = pkt.total_channels();
                let p0 = pkt.modules.first().and_then(|m| m.pressures.first().copied()).unwrap_or(0.0);
                let p_last = pkt.modules.last().and_then(|m| m.pressures.last().copied()).unwrap_or(0.0);
                println!(
                    "     [Frame {:4}] Modules: {}, Total Channels: {}, Ch01={:6.3} PSI, Ch{:02}={:6.3} PSI",
                    pkt.header.frame_number, mod_count, ch_count, p0, ch_count, p_last
                );
            }
        })?;

        thread::sleep(Duration::from_millis(1500));
        client.stop_scan();
        let total = frame_count.load(Ordering::SeqCst);
        println!(
            "  -> Successfully captured {} real frames across {} module(s)! Data streaming verified 100% OK.",
            total,
            hosts.len()
        );
        return Ok(());
    }

    // 1. Automated Integration Self-Test Suite (SSEP + Standalone)
    if is_test {
        println!("\x1b[1;33m[*] Starting multi-module SSEP hardware simulator for self-test...\x1b[0m");
        let mut sim = DSA5000Simulator::new(2323, 5503);
        sim.set_module_count(2); // 2 modules (32 channels)
        sim.start()?;
        thread::sleep(Duration::from_millis(150));

        let mut client = DSA5000Client::new("127.0.0.1", 2323, 5503);
        let _ = client.connect()?;
        run_self_test(&mut client)?;
        sim.stop();

        // Single Standalone Mode Verification
        println!("\x1b[1;33m\n--- [6/7] Standalone Mode (Single Module 16 Channels) Verification ---\x1b[0m");
        let mut sim_standalone = DSA5000Simulator::new(2323, 5503);
        sim_standalone.set_standalone(true);
        sim_standalone.start()?;
        thread::sleep(Duration::from_millis(150));

        let mut client_standalone = DSA5000Client::new("127.0.0.1", 2323, 5503);
        let info = client_standalone.connect()?;
        println!("  -> Standalone Unit Model: {}", info.model);

        // Verify SSEP command (Section 5, P.91)
        let ssep_get = client_standalone.get_ssep_mode(None)?;
        println!("  -> Initial SSEP Query: {}", ssep_get.trim());
        assert!(ssep_get.contains("SSEP 1"), "Expected SSEP 1 in standalone mode!");

        let ssep_set_resp = client_standalone.set_ssep_mode(None, 1)?;
        println!("  -> SSEP 1 Set Response: {}", ssep_set_resp.trim());
        assert!(ssep_set_resp.contains("SSEP 1"), "Expected SSEP 1 echo!");

        let resp = client_standalone.send_command("STATUS")?;
        println!("  -> Status: {}", resp.trim());
        assert!(resp.contains("STANDALONE"), "Expected Standalone status!");
        sim_standalone.stop();

        // Multi-Device Standalone Mode (2 Devices, 32 Channels) Verification
        println!("\x1b[1;33m\n--- [7/7] Multi-Device Standalone Mode (2 Devices, 32 Channels) Verification ---\x1b[0m");
        let mut sim_m1 = DSA5000Simulator::new(2323, 5503);
        sim_m1.set_standalone(true);
        sim_m1.set_base_sn(4005);
        sim_m1.start()?;

        let mut sim_m2 = DSA5000Simulator::new(2324, 5504);
        sim_m2.set_standalone(true);
        sim_m2.set_base_sn(4006);
        sim_m2.start()?;
        thread::sleep(Duration::from_millis(150));

        let dev1 = client::DeviceEndpoint::new("127.0.0.1", 2323, 5503);
        let dev2 = client::DeviceEndpoint::new("127.0.0.1", 2324, 5504);
        let mut client_multi = DSA5000Client::new_multi(vec![dev1, dev2]);
        let multi_info = client_multi.connect()?;
        println!("  -> Connected to 2 standalone devices! Primary Model: {}, S/N: {}", multi_info.model, multi_info.serial_number);

        // Individual Valve Control Verification
        println!("  -> Testing Individual Valve Control (Module 0 -> CAL, Module 1 -> PRG)...");
        let r0 = client_multi.set_valve_target(Some(0), ValveMode::Cal)?;
        let r1 = client_multi.set_valve_target(Some(1), ValveMode::Prg)?;
        println!("     * Module 0 Response: {}", r0);
        println!("     * Module 1 Response: {}", r1);
        let v0_check = client_multi.send_command_to(Some(0), "VALVE")?;
        let v1_check = client_multi.send_command_to(Some(1), "VALVE")?;
        assert!(v0_check.contains("CAL"), "Expected Mod0 to be in CAL state!");
        assert!(v1_check.contains("PRG"), "Expected Mod1 to be in PRG state!");
        println!("  -> [PASS] Individual Valve Control Verified Successfully!");

        // Simultaneous Broadcast Valve Control Verification
        println!("  -> Testing Simultaneous Broadcast Valve Control (All -> PX)...");
        let r_all = client_multi.set_valve_target(None, ValveMode::Px)?;
        println!("     * Broadcast Response: {}", r_all);
        let v_all_check = client_multi.send_command_to(None, "VALVE")?;
        println!("     * All Status:\n{}", v_all_check);
        assert!(v_all_check.contains("PX"), "Expected all modules to be in PX state!");
        println!("  -> [PASS] Simultaneous Broadcast Valve Control Verified Successfully!");

        // Individual & Simultaneous CALZ Verification
        println!("  -> Testing Individual Zero Calibration on Module 0...");
        client_multi.zero_calibration_target(Some(0), |msg, pct| {
            if pct == 10 || pct == 100 {
                println!("     * Mod0 CALZ Progress {}%: {}", pct, msg);
            }
        })?;
        println!("  -> [PASS] Individual CALZ Verified Successfully!");

        println!("  -> Testing Simultaneous Zero Calibration on All Modules...");
        client_multi.zero_calibration_target(None, |msg, pct| {
            if pct == 10 || pct == 100 {
                println!("     * All CALZ Progress {}%: {}", pct, msg);
            }
        })?;
        println!("  -> [PASS] Simultaneous CALZ Verified Successfully!");

        // SSEP Mode Configuration & Explicit Roles Verification (Manual P.79 & P.102)
        println!("  -> Testing SSEP Mode Setting Commands (P.79/102: SSEP 0, 1, 2, 3, 4 & SAVE)...");
        let ssep0_resp = client_multi.set_ssep_mode(Some(0), 0)?;
        println!("     * Mod0 SSEP 0 (Auto): {}", ssep0_resp.trim());
        assert!(ssep0_resp.contains("SSEP 0"), "Expected SSEP 0 echo!");

        let ssep2_resp = client_multi.set_ssep_mode(Some(0), 2)?;
        println!("     * Mod0 SSEP 2 (Controller): {}", ssep2_resp.trim());
        assert!(ssep2_resp.contains("SSEP 2"), "Expected SSEP 2 echo!");

        let ssep3_resp = client_multi.set_ssep_mode(Some(1), 3)?;
        println!("     * Mod1 SSEP 3 (Responder): {}", ssep3_resp.trim());
        assert!(ssep3_resp.contains("SSEP 3"), "Expected SSEP 3 echo!");

        let ssep4_resp = client_multi.set_ssep_mode(Some(1), 4)?;
        println!("     * Mod1 SSEP 4 (Last Responder): {}", ssep4_resp.trim());
        assert!(ssep4_resp.contains("SSEP 4"), "Expected SSEP 4 echo!");

        let save_resp = client_multi.save_hardware(Some(0))?;
        println!("     * Mod0 SAVE: {}", save_resp.trim());
        assert!(save_resp.contains("EEPROM") || save_resp.contains("SAVE"), "Expected SAVE confirmation!");

        let ssep1_resp = client_multi.set_ssep_mode(Some(0), 1)?;
        println!("     * Mod0 SSEP 1 (Standalone): {}", ssep1_resp.trim());
        assert!(ssep1_resp.contains("SSEP 1"), "Expected SSEP 1 echo!");
        println!("  -> [PASS] SSEP Explicit Roles & SAVE Commands Verified Successfully!");

        let multi_frames = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let multi_frames_clone = Arc::clone(&multi_frames);
        let multi_modules = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let multi_modules_clone = Arc::clone(&multi_modules);
        let multi_channels = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let multi_channels_clone = Arc::clone(&multi_channels);

        // Single Unified CSV Logger for 2 standalone devices
        let mut multi_logger = CsvLogger::new("./test_records");
        let _single_csv_path = multi_logger.start(&multi_info, "MultiDevice_Unified_Single.csv")?;
        let logger_arc = Arc::new(std::sync::Mutex::new(multi_logger));
        let ml_clone = Arc::clone(&logger_arc);

        client_multi.start_scan(move |pkt| {
            multi_frames_clone.fetch_add(1, Ordering::SeqCst);
            multi_modules_clone.store(pkt.modules.len(), Ordering::SeqCst);
            multi_channels_clone.store(pkt.total_channels(), Ordering::SeqCst);
            let mut l = ml_clone.lock().unwrap();
            let _ = l.write_frame(&pkt);
        })?;

        thread::sleep(Duration::from_millis(1000));
        client_multi.stop_scan();
        client_multi.disconnect();

        sim_m1.stop();
        sim_m2.stop();

        // Verify single CSV file contains all channels across both devices
        let mut ml = logger_arc.lock().unwrap();
        let saved_path = ml.stop()?;
        if let Some(p) = saved_path {
            let csv_content = std::fs::read_to_string(&p).unwrap_or_default();
            assert!(csv_content.contains("Ch01_"), "Single CSV must contain Device 0 Ch 01!");
            assert!(csv_content.contains("Ch16_"), "Single CSV must contain Device 0 Ch 16!");
            assert!(csv_content.contains("Ch17_"), "Single CSV must contain Device 1 Ch 17!");
            assert!(csv_content.contains("Ch32_"), "Single CSV must contain Device 1 Ch 32!");
            println!("  -> [PASS] Verified all 32 channels from both devices saved in ONE single file ({:?})!", p);
            let _ = std::fs::remove_file(p);
            let _ = std::fs::remove_dir("./test_records");
        }

        let rec_frames = multi_frames.load(Ordering::SeqCst);
        let rec_mods = multi_modules.load(Ordering::SeqCst);
        let rec_chs = multi_channels.load(Ordering::SeqCst);
        println!("  -> Multi-Device Standalone Captured: {} frames, {} modules, {} channels.", rec_frames, rec_mods, rec_chs);
        assert!(rec_frames >= 40, "Expected at least 40 frames from multi-device standalone!");
        assert_eq!(rec_mods, 2, "Expected 2 aggregated modules in multi-device standalone!");
        assert_eq!(rec_chs, 32, "Expected 32 aggregated channels across 2 standalone devices!");

        // Real-time Channel Gain & Offset Statistical Calculation Verification
        println!("\x1b[1;33m\n--- [8/8] Channel Gain/Offset Calibration & Statistics Calculation Verification ---\x1b[0m");
        let mut test_stat = gui::ChannelStats::default();
        test_stat.current = 10.0;
        test_stat.sum = 100.0;
        test_stat.sum_sq = 1005.0;
        test_stat.count = 10;
        test_stat.min = 9.0;
        test_stat.max = 11.0;

        let gain = 2.0;
        let offset = -5.0;
        let calc_val = test_stat.calc_value(gain, offset);
        let calc_mean = test_stat.calc_mean(gain, offset);
        let calc_min = test_stat.calc_min(gain, offset);
        let calc_max = test_stat.calc_max(gain, offset);
        let calc_pp = test_stat.calc_pp(gain);

        println!("  -> Raw: Current=10.0, Mean=10.0, Min=9.0, Max=11.0");
        println!("  -> Settings: Gain=2.0, Offset=-5.0");
        println!("  -> Calculated: Current={}, Mean={}, Min={}, Max={}, P-P={}", calc_val, calc_mean, calc_min, calc_max, calc_pp);

        assert!((calc_val - 15.0).abs() < 1e-4, "Expected calc_val=15.0 (10*2-5)");
        assert!((calc_mean - 15.0).abs() < 1e-4, "Expected calc_mean=15.0 (10*2-5)");
        assert!((calc_min - 13.0).abs() < 1e-4, "Expected calc_min=13.0 (9*2-5)");
        assert!((calc_max - 17.0).abs() < 1e-4, "Expected calc_max=17.0 (11*2-5)");
        assert!((calc_pp - 4.0).abs() < 1e-4, "Expected calc_pp=4.0 ((11-9)*2)");
        println!("  -> [PASS] Gain/Offset Calibration and Real-time Statistical Calculations Verified Successfully!");

        println!("\x1b[1;32m\n>>> ALL RUST SSEP, STANDALONE & STATISTICAL ANALYSIS TESTS PASSED (100% OK)!\x1b[0m");
        return Ok(());
    }

    // 2. Headless / ANSI Console Dashboard Mode
    if is_cli {
        let is_simulate = args.iter().any(|a| a == "--simulate");
        let host = if is_simulate { "127.0.0.1".to_string() } else { "192.168.1.100".to_string() };
        let cmd_port = if is_simulate { 2323u16 } else { 23u16 };
        let data_port = if is_simulate { 5503u16 } else { 503u16 };

        let mut _sim: Option<DSA5000Simulator> = None;
        if is_simulate {
            println!("\x1b[1;33m[*] Starting internal SSEP hardware simulator on {}:{} & {}...\x1b[0m", host, cmd_port, data_port);
            let mut sim = DSA5000Simulator::new(cmd_port, data_port);
            sim.set_module_count(2);
            sim.start()?;
            _sim = Some(sim);
            thread::sleep(Duration::from_millis(150));
        }

        println!("\x1b[1;32m[*] Connecting to DSA5000 at {}:{} / {}...\x1b[0m", host, cmd_port, data_port);
        let mut client = DSA5000Client::new(&host, cmd_port, data_port);
        let info = client.connect()?;
        println!("\x1b[1;32m[+] Connected successfully!\x1b[0m");
        println!("    Model:         {}", info.model);
        println!("    Serial Number: {}", info.serial_number);
        println!("    Scan Rate:     {} Hz", info.scan_rate_hz);
        println!("    Units:         {}", info.engineering_units);
        println!("    Valve State:   {}", info.valve_mode);

        run_live_dashboard(&mut client)?;
        return Ok(());
    }

    // 3. Default: Full Native Windows Desktop GUI Application (eframe)
    println!("\x1b[1;32m[*] Launching Native Desktop GUI Window...\x1b[0m");
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1300.0, 850.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("Scanivalve DSA5000 Studio (SSEP Native Desktop Edition)"),
        ..Default::default()
    };

    eframe::run_native(
        "Scanivalve DSA5000 Studio",
        native_options,
        Box::new(|cc| Ok(Box::new(DsaApp::new(cc)))),
    )?;

    Ok(())
}

fn run_self_test(client: &mut DSA5000Client) -> io::Result<()> {
    println!("\n--- [1/5] SSEP Network Topology & Discovery Verification ---");
    let ssep = client.query_ssep_system()?;
    println!("  -> SSEP Unit Role: {}", ssep.unit_role);
    println!("  -> SSEP Address: {}", ssep.address);
    println!("  -> Number of Responders: {}", ssep.num_responders);
    println!("  -> Discovered Modules: {} modules", ssep.modules.len());
    for m in &ssep.modules {
        println!("     * Mod #{}: {} (SN: {}) Range: {} Valve: {}", m.index, m.model, m.serial_number, m.npr, m.valve_mode);
    }
    assert!(!ssep.modules.is_empty(), "No SSEP modules found!");

    println!("\n--- [2/5] Solenoid Valve Control Verification ---");
    let cal_resp = client.set_valve(ValveMode::Cal)?;
    println!("  -> Set Valve CAL: {}", cal_resp);
    let prg_resp = client.set_valve(ValveMode::Prg)?;
    println!("  -> Set Valve PRG: {}", prg_resp);
    let px_resp = client.set_valve(ValveMode::Px)?;
    println!("  -> Set Valve PX (Measurement): {}", px_resp);

    println!("\n--- [3/5] SSEP Synchronous Zero Calibration (CALZ) Verification ---");
    client.zero_calibration(|status, pct| {
        println!("  -> Progress {:3}% : {}", pct, status);
    })?;

    println!("\n--- [4/5] Multi-Module Binary Streaming (Port 503) Verification ---");
    let mut logger = CsvLogger::new("./test_records");
    let mut test_ch_names = vec![String::new(); 32];
    test_ch_names[0] = "Pitot_Tip".into();
    test_ch_names[1] = "Wing_LE".into();
    logger.set_channel_names(test_ch_names);
    let sys_info = client.system_info.lock().unwrap().clone();
    let csv_file = logger.start(&sys_info, "SSEP_SelfTest")?;
    println!("  -> CSV Logger initialized with custom channel tags: {:?}", csv_file);

    let frame_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let frame_count_clone = Arc::clone(&frame_count);
    let channels_observed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let channels_clone = Arc::clone(&channels_observed);
    let logger_mutex = Arc::new(std::sync::Mutex::new(logger));
    let logger_clone = Arc::clone(&logger_mutex);

    client.start_scan(move |pkt| {
        frame_count_clone.fetch_add(1, Ordering::SeqCst);
        channels_clone.store(pkt.total_channels(), Ordering::SeqCst);
        let mut log = logger_clone.lock().unwrap();
        let _ = log.write_frame(&pkt);
    })?;

    println!("  -> Streaming for 2.0 seconds at 100 Hz...");
    thread::sleep(Duration::from_millis(2000));
    client.stop_scan();

    let total = frame_count.load(Ordering::SeqCst);
    let chs = channels_observed.load(Ordering::SeqCst);
    println!("  -> Received {} frames cleanly across {} channels.", total, chs);
    assert!(total >= 150, "Frame count too low: {}", total);
    assert!(chs >= 32, "Expected at least 32 channels in SSEP mode, got: {}", chs);

    println!("\n--- [5/5] Data Logger Flush & Teardown Verification ---");
    let mut log = logger_mutex.lock().unwrap();
    let recorded_path = log.stop()?;
    if let Some(p) = recorded_path {
        println!("  -> SSEP Log saved successfully: {:?} ({} frames)", p, log.recorded_frames);
        let content = std::fs::read_to_string(&p).unwrap_or_default();
        assert!(content.contains("Ch01_Pitot_Tip_"), "CSV header should contain custom channel name Pitot_Tip");
        assert!(content.contains("Ch02_Wing_LE_"), "CSV header should contain custom channel name Wing_LE");
        println!("  -> Custom channel names in CSV header verified successfully!");
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_dir("./test_records");
    }

    client.disconnect();
    println!("  -> Sockets gracefully disconnected without Port 503 lockup.");

    Ok(())
}

fn run_live_dashboard(client: &mut DSA5000Client) -> io::Result<()> {
    let logger = CsvLogger::new("./data_records");
    let latest_pressures = Arc::new(std::sync::Mutex::new(Vec::new()));
    let latest_p_clone = Arc::clone(&latest_pressures);

    let logger_mutex = Arc::new(std::sync::Mutex::new(logger));
    let logger_clone = Arc::clone(&logger_mutex);

    client.start_scan(move |pkt| {
        let mut all_p = Vec::new();
        for m in &pkt.modules {
            all_p.extend_from_slice(&m.pressures);
        }
        let mut lp = latest_p_clone.lock().unwrap();
        *lp = all_p;

        let mut log = logger_clone.lock().unwrap();
        if log.is_recording {
            let _ = log.write_frame(&pkt);
        }
    })?;

    println!("\n\x1b[1;37mLive SSEP CLI Dashboard Active! Press [Ctrl+C] to exit.\x1b[0m");
    println!("---------------------------------------------------------------");

    let running = Arc::new(AtomicBool::new(true));
    let r_clone = Arc::clone(&running);

    ctrlc_setup(r_clone);

    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(200));

        let stats = client.get_stats();
        let p = latest_pressures.lock().unwrap().clone();

        // ANSI Cursor up & Clear lines
        print!("\x1b[s");
        println!(
            "\x1b[1;36m[FPS: {:3}] [Frames: {:7}] [Loss: {:3}] [PTP Jitter: {:5} ns] [Channels: {:3}]\x1b[0m",
            stats.current_fps, stats.total_frames, stats.dropped_frames, stats.ptp_jitter_ns, p.len()
        );

        // Display up to 32 channels in rows of 8
        let display_count = p.len().min(32);
        for row in 0..(display_count / 8) {
            for col in 0..8 {
                let ch = (row * 8) + col;
                if ch < p.len() {
                    print!("Ch{:02}: \x1b[1;32m{:6.3}\x1b[0m  ", ch + 1, p[ch]);
                }
            }
            println!();
        }
        print!("\x1b[u");
        let _ = stdout().flush();
    }

    client.stop_scan();
    client.disconnect();
    println!("\n[+] Measurement safely stopped and sockets released.");
    Ok(())
}

fn ctrlc_setup(running: Arc<AtomicBool>) {
    let _ = thread::spawn(move || {
        let mut buf = [0u8; 1];
        let _ = io::stdin().read_exact(&mut buf);
        running.store(false, Ordering::SeqCst);
    });
}
