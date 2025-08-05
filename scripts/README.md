# Scripts

This directory contains testing and benchmarking scripts for the AKA timing instrumentation system.

## Scripts Overview

### test-timing-instrumentation.sh
Automated test script that validates the timing instrumentation functionality.

**Purpose:**
- Validates timing data collection
- Tests summary generation
- Verifies CSV export functionality
- Runs performance comparisons

**Usage:**
```bash
./scripts/test-timing-instrumentation.sh
```

**What it tests:**
- ✅ Timing data collection from daemon and direct modes
- ✅ `aka daemon --timing-summary` command
- ✅ `aka daemon --export-timing` command
- ✅ Performance comparison between modes
- ✅ Data persistence across process invocations

### benchmark-daemon-vs-fallback.py
Comprehensive Python benchmarking framework for performance analysis.

**Purpose:**
- Multi-iteration performance testing
- Statistical analysis of timing data
- Custom query testing
- Report generation

**Usage:**
```bash
# Basic benchmark
python3 scripts/benchmark-daemon-vs-fallback.py

# Custom parameters
python3 scripts/benchmark-daemon-vs-fallback.py \
    --iterations 20 \
    --queries "ls" "cat test.txt" "grep pattern file.log"

# With specific config
python3 scripts/benchmark-daemon-vs-fallback.py \
    --config ~/.config/aka/custom.yml \
    --iterations 50
```

**Features:**
- 📊 Wall-clock timing measurements
- 🔄 Automatic daemon start/stop management
- 📈 Statistical analysis (averages, samples)
- 📄 CSV data export
- 🎯 Custom query support

## Requirements

### test-timing-instrumentation.sh
- Bash shell
- `bc` command (for calculations)
- Built AKA binary (`cargo build --release`)

### benchmark-daemon-vs-fallback.py
- Python 3.6+
- Built AKA binary
- Standard library only (no external dependencies)

## Integration with Documentation

These scripts are referenced in:
- `docs/timing-instrumentation.md` - Implementation guide
- `docs/performance-analysis.md` - Performance analysis
- `docs/final-benchmark-report.md` - Benchmark results

## Example Workflow

```bash
# 1. Build the project
cargo build --release

# 2. Run validation tests
./scripts/test-timing-instrumentation.sh

# 3. Run comprehensive benchmark
python3 scripts/benchmark-daemon-vs-fallback.py --iterations 20

# 4. View results
aka daemon --timing-summary
aka daemon --export-timing > results.csv
```

## Output Files

Scripts may generate temporary files:
- `timing_summary.txt` - Summary output (cleaned up automatically)
- `timing_data.csv` - CSV export (cleaned up automatically)
- `build.log` - Build output (cleaned up automatically)
- `aka_timing_data_*.csv` - Benchmark results (preserved)

All temporary files are automatically cleaned up on script exit.