# Timing Instrumentation Guide

## Overview

The AKA codebase includes comprehensive timing instrumentation to measure performance differences between daemon and fallback execution paths. This guide covers implementation details, usage, and analysis.

## Quick Start

### Basic Usage
```bash
# Build and install
cargo build --release
cargo install --path .

# Generate timing data
aka daemon --start
aka query "ls -la"  # Daemon mode
aka daemon --stop
aka query "ls -la"  # Direct mode

# View performance summary
aka daemon --timing-summary

# Export detailed data
aka daemon --export-timing > timing-data.csv
```

### Sample Output
```
📊 TIMING SUMMARY
================
👹 Daemon mode:
   Average: 0.404ms
   Samples: 8
📥 Direct mode:
   Average: 1.774ms
   Samples: 8
⚡ Performance:
   Daemon is 1.370ms faster (77.2% improvement)
```

## Implementation Details

### Core Framework

The timing system consists of three main components:

1. **TimingCollector** - Phase-aware timing collection
2. **TimingData** - Structured timing storage
3. **Global Storage** - Persistent data collection

### Timing Phases

| Phase | Description | Daemon | Direct |
|-------|-------------|---------|--------|
| Total | End-to-end execution | ✅ | ✅ |
| Config Loading | YAML parsing | ❌ (cached) | ✅ |
| IPC Communication | Unix socket | ✅ | ❌ |
| Processing | Alias expansion | ✅ | ✅ |
| Overhead | Startup costs | ✅ | ✅ |

### Code Integration

**Library Framework (`src/lib.rs`):**
```rust
pub struct TimingCollector {
    start_time: Instant,
    config_start: Option<Instant>,
    ipc_start: Option<Instant>,
    processing_start: Option<Instant>,
    mode: ProcessingMode,
}
```

**CLI Integration (`src/bin/aka.rs`):**
```rust
fn handle_command_via_daemon_only_timed(opts: &AkaOpts, timing: &mut TimingCollector) -> Result<i32>
fn handle_command_direct_timed(opts: &AkaOpts, timing: &mut TimingCollector) -> Result<i32>
```

## Data Storage

### Persistent Storage
- **Location**: `~/.config/aka/timing-data.csv`
- **Format**: CSV with timestamps and mode indicators
- **Retention**: Last 1000 entries (automatic cleanup)
- **Thread Safety**: Mutex-protected global storage

### CSV Format
```csv
timestamp,mode,total_ms,config_ms,ipc_ms,processing_ms
1751830762585,Daemon,0.622,0.000,0.613,0.614
1751830771020,Direct,2.150,2.149,0.000,0.393
```

## Performance Analysis

### Key Findings

1. **Daemon is 77.2% faster** (0.404ms vs 1.774ms)
2. **Config loading is the primary bottleneck** (1.77ms in direct mode)
3. **IPC overhead is minimal** (0.39ms)
4. **Process startup dominates wall-clock time** (~66ms)

### Detailed Breakdown

#### Daemon Mode (0.404ms average)
- IPC Communication: 0.390ms (96.5%)
- Processing: 0.390ms (overlapped)
- Config Loading: 0ms (pre-loaded)
- Overhead: 0.014ms (3.5%)

#### Direct Mode (1.774ms average)
- Config Loading: 1.774ms (99.9%)
- Processing: 0.370ms (overlapped)
- IPC Communication: 0ms
- Overhead: 0.001ms (0.1%)

### Wall-Clock vs Internal Timing

| Component | Time | Percentage |
|-----------|------|------------|
| Process Startup | ~66ms | 98.5% |
| Config Loading (Direct) | 1.77ms | 1.3% |
| IPC Communication (Daemon) | 0.39ms | 0.6% |
| Alias Processing | 0.37ms | 0.6% |

## Usage Patterns

### Performance by Usage Type

| Usage Pattern | Queries/sec | Daemon Advantage | Time Saved |
|---------------|-------------|------------------|------------|
| Interactive CLI | 1-2 | 77.2% | 1.37ms |
| ZLE Integration | 10-50 | 77.2% | 13.7-68.5ms |
| Batch Scripts | 100+ | 77.2% | 137ms+ |

### Recommendations

1. **Interactive Usage**: Daemon provides noticeable improvement for frequent users
2. **High-Frequency Usage**: Essential for ZLE integration and scripts
3. **Batch Operations**: Linear scaling makes daemon crucial for performance

## Testing and Validation

### Automated Testing
The timing system includes comprehensive test coverage:

```bash
# Run validation tests
./scripts/test-timing-instrumentation.sh

# Run performance benchmarks
python3 scripts/benchmark-daemon-vs-fallback.py
```

### Manual Testing
```bash
# Generate test data
for i in {1..10}; do
    aka query "ls -la"
done

# Check results
aka daemon --timing-summary
```

## Optimization Opportunities

### Immediate Improvements
1. **Config Caching**: Cache parsed YAML in direct mode
2. **Binary Protocol**: Replace JSON with MessagePack
3. **Connection Pooling**: Reuse socket connections

### Future Enhancements
1. **Shared Memory**: For large configuration data
2. **Batch Processing**: Multiple queries per request
3. **Binary Optimization**: Reduce startup time

## Troubleshooting

### Common Issues

**No timing data collected:**
- Ensure queries are being run after instrumentation
- Check that daemon is properly started/stopped for mode testing

**Timing summary shows 0 samples:**
- Run some queries first to generate data
- Verify persistent storage is working: `ls ~/.config/aka/timing-data.csv`

**CSV export is empty:**
- Check file permissions on `~/.config/aka/`
- Ensure timing data has been generated

### Debug Commands
```bash
# Check daemon status
aka daemon --status

# Verify timing file exists
ls -la ~/.config/aka/timing-data.csv

# Check recent timing data
tail ~/.config/aka/timing-data.csv
```

## API Reference

### CLI Commands
- `aka daemon --timing-summary` - Show performance summary
- `aka daemon --export-timing` - Export CSV data

### Internal Functions
- `TimingCollector::new(mode)` - Create timing collector
- `log_timing(timing_data)` - Store timing data
- `get_timing_summary()` - Get performance statistics
- `export_timing_csv()` - Export CSV data 