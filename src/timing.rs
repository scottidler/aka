use eyre::{eyre, Result};
use log::debug;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::{Instant, Duration, SystemTime};


// Global timing storage for analysis
use std::sync::Mutex;
use lazy_static::lazy_static;

// Import ProcessingMode from lib.rs
use crate::ProcessingMode;

lazy_static! {
    static ref TIMING_LOG: Mutex<Vec<TimingData>> = Mutex::new(Vec::new());
}

// Check if benchmark mode is enabled
pub fn is_benchmark_mode() -> bool {
    std::env::var("AKA_BENCHMARK").is_ok() ||
    std::env::var("AKA_TIMING").is_ok() ||
    std::env::var("AKA_DEBUG_TIMING").is_ok()
}

// Timing instrumentation framework
#[derive(Debug, Clone)]
pub struct TimingData {
    pub total_duration: Duration,
    pub config_load_duration: Option<Duration>,
    pub ipc_duration: Option<Duration>,
    pub processing_duration: Duration,
    pub mode: ProcessingMode,
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone)]
pub struct TimingCollector {
    start_time: Instant,
    config_start: Option<Instant>,
    ipc_start: Option<Instant>,
    processing_start: Option<Instant>,
    mode: ProcessingMode,
}

impl TimingCollector {
    pub fn new(mode: ProcessingMode) -> Self {
        TimingCollector {
            start_time: Instant::now(),
            config_start: None,
            ipc_start: None,
            processing_start: None,
            mode,
        }
    }

    pub fn start_config_load(&mut self) {
        self.config_start = Some(Instant::now());
    }

    pub fn end_config_load(&mut self) -> Option<Duration> {
        self.config_start.map(|start| start.elapsed())
    }

    pub fn start_ipc(&mut self) {
        self.ipc_start = Some(Instant::now());
    }

    pub fn end_ipc(&mut self) -> Option<Duration> {
        self.ipc_start.map(|start| start.elapsed())
    }

    pub fn start_processing(&mut self) {
        self.processing_start = Some(Instant::now());
    }

    pub fn end_processing(&mut self) -> Duration {
        self.processing_start.map(|start| start.elapsed()).unwrap_or_default()
    }

    pub fn finalize(self) -> TimingData {
        TimingData {
            total_duration: self.start_time.elapsed(),
            config_load_duration: self.config_start.map(|start| start.elapsed()),
            ipc_duration: self.ipc_start.map(|start| start.elapsed()),
            processing_duration: self.processing_start.map(|start| start.elapsed()).unwrap_or_default(),
            mode: self.mode,
            timestamp: SystemTime::now(),
        }
    }
}

impl TimingData {
    pub fn log_detailed(&self) {
        // Only log detailed timing if benchmark mode is enabled
        if !is_benchmark_mode() {
            return;
        }

        let emoji = match self.mode {
            ProcessingMode::Daemon => "👹",
            ProcessingMode::Direct => "📥",
        };

        debug!("{} === TIMING BREAKDOWN ({:?}) ===", emoji, self.mode);
        debug!("  🎯 Total execution: {:.3}ms", self.total_duration.as_secs_f64() * 1000.0);

        if let Some(config_duration) = self.config_load_duration {
            debug!("  📋 Config loading: {:.3}ms ({:.1}%)",
                config_duration.as_secs_f64() * 1000.0,
                (config_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
            );
        }

        if let Some(ipc_duration) = self.ipc_duration {
            debug!("  🔌 IPC communication: {:.3}ms ({:.1}%)",
                ipc_duration.as_secs_f64() * 1000.0,
                (ipc_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
            );
        }

        debug!("  ⚙️  Processing: {:.3}ms ({:.1}%)",
            self.processing_duration.as_secs_f64() * 1000.0,
            (self.processing_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
        );

        // Calculate overhead
        let accounted = self.config_load_duration.unwrap_or_default() +
                       self.ipc_duration.unwrap_or_default() +
                       self.processing_duration;
        let overhead = self.total_duration.saturating_sub(accounted);
        debug!("  🏗️  Overhead: {:.3}ms ({:.1}%)",
            overhead.as_secs_f64() * 1000.0,
            (overhead.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
        );
    }

    pub fn to_csv_line(&self) -> String {
        let timestamp_ms = self.timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let config_ms = self.config_load_duration
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let ipc_ms = self.ipc_duration
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        format!("{},{:?},{:.3},{:.3},{:.3},{:.3}",
            timestamp_ms,
            self.mode,
            self.total_duration.as_secs_f64() * 1000.0,
            config_ms,
            ipc_ms,
            self.processing_duration.as_secs_f64() * 1000.0
        )
    }
}

fn parse_csv_line(line: &str) -> Result<TimingData> {
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() != 6 {
        return Err(eyre!("Invalid CSV line format"));
    }

    let timestamp_ms: u64 = parts[0].parse()?;
    let mode = match parts[1] {
        "Daemon" => ProcessingMode::Daemon,
        "Direct" => ProcessingMode::Direct,
        _ => return Err(eyre!("Invalid processing mode")),
    };

    let total_ms: f64 = parts[2].parse()?;
    let config_ms: f64 = parts[3].parse()?;
    let ipc_ms: f64 = parts[4].parse()?;
    let processing_ms: f64 = parts[5].parse()?;

    Ok(TimingData {
        total_duration: Duration::from_millis(total_ms as u64),
        config_load_duration: if config_ms > 0.0 { Some(Duration::from_millis(config_ms as u64)) } else { None },
        ipc_duration: if ipc_ms > 0.0 { Some(Duration::from_millis(ipc_ms as u64)) } else { None },
        processing_duration: Duration::from_millis(processing_ms as u64),
        mode,
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_millis(timestamp_ms),
    })
}

pub fn log_timing(timing: TimingData) {
    // Only log detailed breakdown if benchmark mode is enabled
    if is_benchmark_mode() {
        timing.log_detailed();
    }

    // Always store in memory for CLI commands (minimal overhead)
    if let Ok(mut log) = TIMING_LOG.lock() {
        log.push(timing.clone());

        // Keep only last 1000 entries to prevent memory bloat
        let len = log.len();
        if len > 1000 {
            log.drain(0..len - 1000);
        }
    }

    // Only write to CSV file if benchmark mode is enabled
    if is_benchmark_mode() {
        if let Ok(timing_file_path) = get_timing_file_path() {
            // Ensure directory exists
            if let Some(parent) = timing_file_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let csv_line = timing.to_csv_line();
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(timing_file_path) {
                use std::io::Write;
                let _ = writeln!(file, "{}", csv_line);
            }
        }
    }
}

pub fn export_timing_csv() -> Result<String> {
    let mut csv = String::from("timestamp,mode,total_ms,config_ms,ipc_ms,processing_ms\n");

    // Load from persistent file if it exists
    if let Ok(timing_file_path) = get_timing_file_path() {
        if let Ok(content) = std::fs::read_to_string(timing_file_path) {
            for line in content.lines() {
                if !line.trim().is_empty() {
                    csv.push_str(line);
                    csv.push('\n');
                }
            }
        }
    }

    // Also include current session data
    if let Ok(log) = TIMING_LOG.lock() {
        for timing in log.iter() {
            csv.push_str(&timing.to_csv_line());
            csv.push('\n');
        }
    }

    Ok(csv)
}

pub fn get_timing_summary() -> Result<(Duration, Duration, usize, usize)> {
    let mut all_timings = Vec::new();

    // Load from persistent file if it exists
    if let Ok(timing_file_path) = get_timing_file_path() {
        if let Ok(content) = std::fs::read_to_string(timing_file_path) {
            for line in content.lines() {
                if let Ok(timing) = parse_csv_line(line) {
                    all_timings.push(timing);
                }
            }
        }
    }

    // Also include current session data
    if let Ok(log) = TIMING_LOG.lock() {
        all_timings.extend(log.iter().cloned());
    }

    let daemon_timings: Vec<_> = all_timings.iter().filter(|t| matches!(t.mode, ProcessingMode::Daemon)).collect();
    let direct_timings: Vec<_> = all_timings.iter().filter(|t| matches!(t.mode, ProcessingMode::Direct)).collect();

    let daemon_avg = if !daemon_timings.is_empty() {
        daemon_timings.iter().map(|t| t.total_duration).sum::<Duration>() / daemon_timings.len() as u32
    } else {
        Duration::default()
    };

    let direct_avg = if !direct_timings.is_empty() {
        direct_timings.iter().map(|t| t.total_duration).sum::<Duration>() / direct_timings.len() as u32
    } else {
        Duration::default()
    };

    Ok((daemon_avg, direct_avg, daemon_timings.len(), direct_timings.len()))
}

pub fn get_timing_file_path() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre!("Could not determine local data directory"))?
        .join("aka");
    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("timing_data.csv"))
}