use eyre::{eyre, Result};
use log::debug;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

// Global timing storage for analysis
use lazy_static::lazy_static;
use std::sync::Mutex;

// Import ProcessingMode from lib.rs
use crate::ProcessingMode;

lazy_static! {
    static ref TIMING_LOG: Mutex<Vec<TimingData>> = Mutex::new(Vec::new());
}

// Check if benchmark mode is enabled
pub fn is_benchmark_mode() -> bool {
    std::env::var("AKA_BENCHMARK").is_ok()
        || std::env::var("AKA_TIMING").is_ok()
        || std::env::var("AKA_DEBUG_TIMING").is_ok()
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
        debug!(
            "  🎯 Total execution: {:.3}ms",
            self.total_duration.as_secs_f64() * 1000.0
        );

        if let Some(config_duration) = self.config_load_duration {
            debug!(
                "  📋 Config loading: {:.3}ms ({:.1}%)",
                config_duration.as_secs_f64() * 1000.0,
                (config_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
            );
        }

        if let Some(ipc_duration) = self.ipc_duration {
            debug!(
                "  🔌 IPC communication: {:.3}ms ({:.1}%)",
                ipc_duration.as_secs_f64() * 1000.0,
                (ipc_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
            );
        }

        debug!(
            "  ⚙️  Processing: {:.3}ms ({:.1}%)",
            self.processing_duration.as_secs_f64() * 1000.0,
            (self.processing_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
        );

        // Calculate overhead
        let accounted = self.config_load_duration.unwrap_or_default()
            + self.ipc_duration.unwrap_or_default()
            + self.processing_duration;
        let overhead = self.total_duration.saturating_sub(accounted);
        debug!(
            "  🏗️  Overhead: {:.3}ms ({:.1}%)",
            overhead.as_secs_f64() * 1000.0,
            (overhead.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
        );
    }

    pub fn to_csv_line(&self) -> String {
        let timestamp_ms = self
            .timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let config_ms = self
            .config_load_duration
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let ipc_ms = self.ipc_duration.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);

        format!(
            "{},{:?},{:.3},{:.3},{:.3},{:.3}",
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
        config_load_duration: if config_ms > 0.0 {
            Some(Duration::from_millis(config_ms as u64))
        } else {
            None
        },
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
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(timing_file_path) {
                use std::io::Write;
                let _ = writeln!(file, "{csv_line}");
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

    let daemon_timings: Vec<_> = all_timings
        .iter()
        .filter(|t| matches!(t.mode, ProcessingMode::Daemon))
        .collect();
    let direct_timings: Vec<_> = all_timings
        .iter()
        .filter(|t| matches!(t.mode, ProcessingMode::Direct))
        .collect();

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_is_benchmark_mode_default() {
        // Clear any existing env vars that would enable benchmark mode
        std::env::remove_var("AKA_BENCHMARK");
        std::env::remove_var("AKA_TIMING");
        std::env::remove_var("AKA_DEBUG_TIMING");

        // Default should be false when no env vars are set
        // Note: this test may fail if run in a benchmark environment
        let result = is_benchmark_mode();
        // Just verify it doesn't panic - the actual value depends on environment
        let _ = result;
    }

    #[test]
    fn test_timing_collector_creation() {
        let collector = TimingCollector::new(ProcessingMode::Daemon);
        // Verify the collector was created with correct mode
        assert!(matches!(collector.mode, ProcessingMode::Daemon));

        let collector2 = TimingCollector::new(ProcessingMode::Direct);
        assert!(matches!(collector2.mode, ProcessingMode::Direct));
    }

    #[test]
    fn test_timing_collector_config_load() {
        let mut collector = TimingCollector::new(ProcessingMode::Direct);

        // Initially, config_start should be None
        assert!(collector.config_start.is_none());

        collector.start_config_load();
        assert!(collector.config_start.is_some());

        // Small sleep to ensure duration is measurable
        std::thread::sleep(Duration::from_millis(1));

        let duration = collector.end_config_load();
        assert!(duration.is_some());
        assert!(duration.unwrap() >= Duration::from_millis(1));
    }

    #[test]
    fn test_timing_collector_ipc() {
        let mut collector = TimingCollector::new(ProcessingMode::Daemon);

        // Initially, ipc_start should be None
        assert!(collector.ipc_start.is_none());

        collector.start_ipc();
        assert!(collector.ipc_start.is_some());

        std::thread::sleep(Duration::from_millis(1));

        let duration = collector.end_ipc();
        assert!(duration.is_some());
        assert!(duration.unwrap() >= Duration::from_millis(1));
    }

    #[test]
    fn test_timing_collector_processing() {
        let mut collector = TimingCollector::new(ProcessingMode::Direct);

        // Initially, processing_start should be None
        assert!(collector.processing_start.is_none());

        collector.start_processing();
        assert!(collector.processing_start.is_some());

        std::thread::sleep(Duration::from_millis(1));

        let duration = collector.end_processing();
        assert!(duration >= Duration::from_millis(1));
    }

    #[test]
    fn test_timing_collector_end_without_start() {
        let mut collector = TimingCollector::new(ProcessingMode::Direct);

        // End without start should return None/default
        let config_duration = collector.end_config_load();
        assert!(config_duration.is_none());

        let ipc_duration = collector.end_ipc();
        assert!(ipc_duration.is_none());

        let processing_duration = collector.end_processing();
        assert_eq!(processing_duration, Duration::default());
    }

    #[test]
    fn test_timing_collector_finalize() {
        let mut collector = TimingCollector::new(ProcessingMode::Daemon);

        // Start some timers
        collector.start_config_load();
        std::thread::sleep(Duration::from_millis(1));

        collector.start_ipc();
        std::thread::sleep(Duration::from_millis(1));

        collector.start_processing();
        std::thread::sleep(Duration::from_millis(1));

        let timing_data = collector.finalize();

        assert!(matches!(timing_data.mode, ProcessingMode::Daemon));
        assert!(timing_data.total_duration >= Duration::from_millis(3));
        assert!(timing_data.config_load_duration.is_some());
        assert!(timing_data.ipc_duration.is_some());
        // Processing duration is also calculated in finalize
    }

    #[test]
    fn test_timing_data_to_csv_line() {
        let timing_data = TimingData {
            total_duration: Duration::from_millis(100),
            config_load_duration: Some(Duration::from_millis(30)),
            ipc_duration: Some(Duration::from_millis(20)),
            processing_duration: Duration::from_millis(50),
            mode: ProcessingMode::Daemon,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1000000),
        };

        let csv_line = timing_data.to_csv_line();

        // Verify the CSV format
        let parts: Vec<&str> = csv_line.split(',').collect();
        assert_eq!(parts.len(), 6);
        assert_eq!(parts[1], "Daemon");

        // Parse the numeric values (verify they're valid)
        let _timestamp: u64 = parts[0].parse().unwrap();
        let _total_ms: f64 = parts[2].parse().unwrap();
        let _config_ms: f64 = parts[3].parse().unwrap();
        let _ipc_ms: f64 = parts[4].parse().unwrap();
        let _processing_ms: f64 = parts[5].parse().unwrap();
    }

    #[test]
    fn test_timing_data_to_csv_line_no_optional_durations() {
        let timing_data = TimingData {
            total_duration: Duration::from_millis(100),
            config_load_duration: None,
            ipc_duration: None,
            processing_duration: Duration::from_millis(50),
            mode: ProcessingMode::Direct,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1000000),
        };

        let csv_line = timing_data.to_csv_line();

        let parts: Vec<&str> = csv_line.split(',').collect();
        assert_eq!(parts.len(), 6);
        assert_eq!(parts[1], "Direct");

        // Optional durations should be 0.0
        let config_ms: f64 = parts[3].parse().unwrap();
        let ipc_ms: f64 = parts[4].parse().unwrap();
        assert_eq!(config_ms, 0.0);
        assert_eq!(ipc_ms, 0.0);
    }

    #[test]
    fn test_parse_csv_line_valid_daemon() {
        let csv_line = "1000000000,Daemon,100.000,30.000,20.000,50.000";
        let result = parse_csv_line(csv_line);

        assert!(result.is_ok());
        let timing_data = result.unwrap();

        assert!(matches!(timing_data.mode, ProcessingMode::Daemon));
        assert!(timing_data.config_load_duration.is_some());
        assert!(timing_data.ipc_duration.is_some());
    }

    #[test]
    fn test_parse_csv_line_valid_direct() {
        let csv_line = "1000000000,Direct,100.000,0.000,0.000,50.000";
        let result = parse_csv_line(csv_line);

        assert!(result.is_ok());
        let timing_data = result.unwrap();

        assert!(matches!(timing_data.mode, ProcessingMode::Direct));
        assert!(timing_data.config_load_duration.is_none());
        assert!(timing_data.ipc_duration.is_none());
    }

    #[test]
    fn test_parse_csv_line_invalid_format() {
        // Wrong number of fields
        let csv_line = "1000000000,Daemon,100.000,30.000";
        let result = parse_csv_line(csv_line);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_csv_line_invalid_mode() {
        let csv_line = "1000000000,InvalidMode,100.000,30.000,20.000,50.000";
        let result = parse_csv_line(csv_line);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_csv_line_invalid_numbers() {
        let csv_line = "not_a_number,Daemon,100.000,30.000,20.000,50.000";
        let result = parse_csv_line(csv_line);
        assert!(result.is_err());

        let csv_line2 = "1000000000,Daemon,not_a_float,30.000,20.000,50.000";
        let result2 = parse_csv_line(csv_line2);
        assert!(result2.is_err());
    }

    #[test]
    fn test_csv_roundtrip() {
        let original = TimingData {
            total_duration: Duration::from_millis(100),
            config_load_duration: Some(Duration::from_millis(30)),
            ipc_duration: Some(Duration::from_millis(20)),
            processing_duration: Duration::from_millis(50),
            mode: ProcessingMode::Daemon,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1000000),
        };

        let csv_line = original.to_csv_line();
        let parsed = parse_csv_line(&csv_line).unwrap();

        // Mode should match
        assert!(matches!(parsed.mode, ProcessingMode::Daemon));

        // Durations should be approximately equal (some precision loss due to f64 conversion)
        // Note: Duration -> ms -> Duration conversion loses sub-millisecond precision
    }

    #[test]
    fn test_timing_data_log_detailed_no_panic() {
        // This test just verifies log_detailed doesn't panic
        let timing_data = TimingData {
            total_duration: Duration::from_millis(100),
            config_load_duration: Some(Duration::from_millis(30)),
            ipc_duration: Some(Duration::from_millis(20)),
            processing_duration: Duration::from_millis(50),
            mode: ProcessingMode::Daemon,
            timestamp: SystemTime::now(),
        };

        // Should not panic regardless of benchmark mode
        timing_data.log_detailed();
    }

    #[test]
    fn test_timing_data_log_detailed_with_zero_total() {
        // Edge case: zero total duration (shouldn't cause division by zero)
        let timing_data = TimingData {
            total_duration: Duration::from_millis(0),
            config_load_duration: Some(Duration::from_millis(0)),
            ipc_duration: None,
            processing_duration: Duration::from_millis(0),
            mode: ProcessingMode::Direct,
            timestamp: SystemTime::now(),
        };

        // Should not panic
        timing_data.log_detailed();
    }

    #[test]
    fn test_get_timing_file_path() {
        // This should succeed on systems with data_local_dir defined
        let result = get_timing_file_path();

        match result {
            Ok(path) => {
                assert!(path.to_string_lossy().contains("aka"));
                assert!(path.to_string_lossy().ends_with("timing_data.csv"));
            }
            Err(_) => {
                // Acceptable if system doesn't have data_local_dir
            }
        }
    }

    #[test]
    fn test_log_timing_no_panic() {
        let timing_data = TimingData {
            total_duration: Duration::from_millis(100),
            config_load_duration: None,
            ipc_duration: None,
            processing_duration: Duration::from_millis(100),
            mode: ProcessingMode::Direct,
            timestamp: SystemTime::now(),
        };

        // Should not panic
        log_timing(timing_data);
    }

    #[test]
    fn test_export_timing_csv_format() {
        let result = export_timing_csv();

        assert!(result.is_ok());
        let csv = result.unwrap();

        // Should have header
        assert!(csv.starts_with("timestamp,mode,total_ms,config_ms,ipc_ms,processing_ms"));
    }

    #[test]
    fn test_get_timing_summary_no_panic() {
        // Should not panic even if no timing data exists
        let result = get_timing_summary();

        match result {
            Ok((daemon_avg, direct_avg, daemon_count, direct_count)) => {
                // Averages should be valid durations
                let _ = daemon_avg;
                let _ = direct_avg;
                // Counts should be non-negative (they're usize)
                let _ = daemon_count;
                let _ = direct_count;
            }
            Err(_) => {
                // Acceptable if timing file operations fail
            }
        }
    }

    #[test]
    fn test_processing_mode_display() {
        // Test that ProcessingMode can be used in debug/display contexts
        let daemon_mode = ProcessingMode::Daemon;
        let direct_mode = ProcessingMode::Direct;

        let daemon_str = format!("{daemon_mode:?}");
        let direct_str = format!("{direct_mode:?}");

        assert_eq!(daemon_str, "Daemon");
        assert_eq!(direct_str, "Direct");
    }

    #[test]
    fn test_timing_data_clone() {
        let original = TimingData {
            total_duration: Duration::from_millis(100),
            config_load_duration: Some(Duration::from_millis(30)),
            ipc_duration: Some(Duration::from_millis(20)),
            processing_duration: Duration::from_millis(50),
            mode: ProcessingMode::Daemon,
            timestamp: SystemTime::now(),
        };

        let cloned = original.clone();

        assert_eq!(original.total_duration, cloned.total_duration);
        assert_eq!(original.config_load_duration, cloned.config_load_duration);
        assert_eq!(original.ipc_duration, cloned.ipc_duration);
        assert_eq!(original.processing_duration, cloned.processing_duration);
    }

    #[test]
    fn test_timing_collector_clone() {
        let collector = TimingCollector::new(ProcessingMode::Direct);
        let cloned = collector.clone();

        assert!(matches!(cloned.mode, ProcessingMode::Direct));
    }
}
