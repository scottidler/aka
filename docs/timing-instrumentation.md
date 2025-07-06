# Timing Instrumentation Guide

## Overview

The AKA codebase includes comprehensive timing instrumentation to measure performance differences between daemon and fallback execution paths. By default, timing collection runs silently in the background. Detailed logging and CSV export are only enabled in benchmark mode.

## Quick Start

### Basic Usage
```bash
# Build and install
cargo build --release
cargo install --path .

# Normal usage (silent timing collection)
aka daemon --start
aka query "ls -la"  # Daemon mode
aka daemon --stop
aka query "ls -la"  # Direct mode

# View performance summary (always available)
aka daemon --timing-summary

# Export detailed data (from in-memory storage)
aka daemon --export-timing > timing-data.csv
```

### Benchmark Mode
For detailed logging and persistent CSV storage, enable benchmark mode:

```bash
# Enable benchmark mode with environment variable
export AKA_BENCHMARK=1
# OR
export AKA_TIMING=1
# OR
export AKA_DEBUG_TIMING=1

# Now timing details will be logged and written to CSV
aka query "ls -la"  # Will show detailed timing breakdown in logs
```

### Sample Output

**Normal Mode (Silent):**
```bash
$ aka query "ls -la"
eza -la
# No timing logs shown
```

**Benchmark Mode:**
```bash
$ AKA_BENCHMARK=1 aka query "ls -la"
eza -la
[2025-07-06T20:07:00Z INFO  aka_lib] 👹 === TIMING BREAKDOWN (Daemon) ===
[2025-07-06T20:07:00Z INFO  aka_lib]   🎯 Total execution: 0.007ms
[2025-07-06T20:07:00Z INFO  aka_lib]   ⚙️  Processing: 0.002ms (35.7%)
[2025-07-06T20:07:00Z INFO  aka_lib]   🏗️  Overhead: 0.004ms (64.3%)
```

## Implementation Details

### Core Framework

The timing system consists of three main components:

1. **TimingCollector** - Phase-aware timing collection
2. **TimingData** - Structured timing storage
3. **Global Storage** - In-memory data collection (always enabled)

### Timing Phases

| Phase | Description | Daemon | Direct |
|-------|-------------|---------|--------|
| Total | End-to-end execution | ✅ | ✅ |
| Config Loading | YAML parsing | ❌ (cached) | ✅ |
| IPC Communication | Unix socket | ✅ | ❌ |
| Processing | Alias expansion | ✅ | ✅ |
| Overhead | Startup costs | ✅ | ✅ |

### Benchmark Mode Control

Timing behavior is controlled by environment variables:

| Mode | Environment Variables | Logging | CSV Export | Memory Storage |
|------|----------------------|---------|------------|----------------|
| **Normal** | None | ❌ Silent | ❌ Disabled | ✅ Enabled |
| **Benchmark** | `AKA_BENCHMARK=1`<br/>`AKA_TIMING=1`<br/>`AKA_DEBUG_TIMING=1` | ✅ Detailed | ✅ Enabled | ✅ Enabled |

### Code Integration

**Library Framework (`src/lib.rs`):**
```rust
// Check if benchmark mode is enabled
fn is_benchmark_mode() -> bool {
    std::env::var("AKA_BENCHMARK").is_ok() ||
    std::env::var("AKA_TIMING").is_ok() ||
    std::env::var("AKA_DEBUG_TIMING").is_ok()
}

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

### In-Memory Storage (Always Enabled)
- **Location**: Global static variable in memory
- **Retention**: Last 1000 entries (automatic cleanup)
- **Thread Safety**: Mutex-protected
- **Performance**: Minimal overhead (~microseconds)

### Persistent CSV Storage (Benchmark Mode Only)
- **Location**: `~/.config/aka/timing-data.csv`
- **Format**: CSV with timestamps and mode indicators
- **Enabled**: Only when `AKA_BENCHMARK=1` (or similar) is set
- **Purpose**: Detailed analysis and external processing

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

1. **Normal Usage**: Timing collection runs silently with minimal overhead
2. **Performance Analysis**: Enable benchmark mode for detailed insights
3. **Development**: Use benchmark mode during optimization work
4. **Production**: Normal mode provides CLI access to timing data without log noise

## Testing and Validation

### Automated Testing
The timing system includes comprehensive test coverage:

```bash
# Run validation tests (will enable benchmark mode automatically)
./scripts/test-timing-instrumentation.sh

# Run performance benchmarks (enables benchmark mode)
AKA_BENCHMARK=1 python3 scripts/benchmark-daemon-vs-fallback.py
```

### Manual Testing
```bash
# Silent collection (normal mode)
for i in {1..10}; do
    aka query "ls -la"
done

# Detailed logging (benchmark mode)
AKA_BENCHMARK=1 aka query "ls -la"

# Check results (always available)
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

**No timing data in summary:**
- Timing collection always runs, check: `aka daemon --timing-summary`
- Data is stored in memory even without benchmark mode

**No detailed logs showing:**
- Enable benchmark mode: `export AKA_BENCHMARK=1`
- Check log level: `export RUST_LOG=info`

**CSV file not created:**
- CSV export only works in benchmark mode
- Enable with: `export AKA_BENCHMARK=1`
- Check file: `ls ~/.config/aka/timing-data.csv`

**Performance seems slow:**
- Normal timing collection has ~microsecond overhead
- Benchmark mode adds logging overhead (~milliseconds)
- Disable benchmark mode for production use

### Debug Commands
```bash
# Check if benchmark mode is active
echo $AKA_BENCHMARK

# Enable benchmark mode temporarily
AKA_BENCHMARK=1 aka query "test"

# Check daemon status
aka daemon --status

# View in-memory timing data
aka daemon --timing-summary

# Export current data (works in both modes)
aka daemon --export-timing
```

## API Reference

### CLI Commands
- `aka daemon --timing-summary` - Show performance summary (always available)
- `aka daemon --export-timing` - Export data as CSV (from memory + file if benchmark mode)

### Environment Variables
- `AKA_BENCHMARK=1` - Enable detailed logging and CSV export
- `AKA_TIMING=1` - Alternative benchmark mode flag
- `AKA_DEBUG_TIMING=1` - Alternative benchmark mode flag

### Internal Functions
- `is_benchmark_mode()` - Check if benchmark mode is enabled
- `TimingCollector::new(mode)` - Create timing collector
- `log_timing(timing_data)` - Store timing data (conditional logging/CSV)
- `get_timing_summary()` - Get performance statistics
- `export_timing_csv()` - Export CSV data
