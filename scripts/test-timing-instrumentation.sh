#!/bin/bash

# Test script for timing instrumentation
# This script validates that timing data collection works correctly

set -e

echo "🧪 Testing AKA Timing Instrumentation"
echo "======================================="

# Enable benchmark mode for testing
export AKA_BENCHMARK=1
export RUST_LOG=info

# Build the project
echo "📦 Building project..."
cargo build --release

# Clean up any existing timing data
TIMING_FILE="$HOME/.config/aka/timing-data.csv"
if [ -f "$TIMING_FILE" ]; then
    echo "🧹 Cleaning up existing timing data..."
    rm "$TIMING_FILE"
fi

# Start daemon for testing
echo "🚀 Starting daemon..."
./target/release/aka daemon --start

# Wait for daemon to be ready
sleep 1

# Test daemon mode (should log timing details)
echo "👹 Testing daemon mode..."
./target/release/aka query "ls -la" >/dev/null
echo "✅ Daemon query completed"

# Stop daemon
echo "🛑 Stopping daemon..."
./target/release/aka daemon --stop

# Wait for daemon to stop
sleep 1

# Test direct mode (should log timing details)
echo "📥 Testing direct mode..."
./target/release/aka query "ls -la" >/dev/null
echo "✅ Direct query completed"

# Test timing summary
echo "📊 Testing timing summary..."
SUMMARY_OUTPUT=$(./target/release/aka daemon --timing-summary)
echo "$SUMMARY_OUTPUT"

# Verify summary contains expected data
if echo "$SUMMARY_OUTPUT" | grep -q "Daemon mode:" && echo "$SUMMARY_OUTPUT" | grep -q "Direct mode:"; then
    echo "✅ Timing summary working correctly"
else
    echo "❌ Timing summary missing expected data"
    exit 1
fi

# Test CSV export
echo "📊 Testing CSV export..."
CSV_OUTPUT=$(./target/release/aka daemon --export-timing)
echo "CSV output preview:"
echo "$CSV_OUTPUT" | head -5

# Verify CSV has header and data
if echo "$CSV_OUTPUT" | grep -q "timestamp,mode,total_ms" && echo "$CSV_OUTPUT" | grep -q "Daemon\|Direct"; then
    echo "✅ CSV export working correctly"
else
    echo "❌ CSV export missing expected data"
    exit 1
fi

# Check persistent file was created (benchmark mode only)
if [ -f "$TIMING_FILE" ]; then
    echo "✅ Persistent timing file created: $TIMING_FILE"
    echo "File contents preview:"
    head -3 "$TIMING_FILE"
else
    echo "❌ Persistent timing file not created"
    exit 1
fi

# Test performance comparison
echo "🏁 Running performance comparison..."
echo "Starting daemon for comparison..."
./target/release/aka daemon --start
sleep 1

# Run multiple queries to get better data
echo "Running daemon queries..."
for i in {1..5}; do
    ./target/release/aka query "ls -la" >/dev/null
done

echo "Stopping daemon..."
./target/release/aka daemon --stop
sleep 1

echo "Running direct queries..."
for i in {1..5}; do
    ./target/release/aka query "ls -la" >/dev/null
done

# Final summary
echo "📈 Final timing summary:"
./target/release/aka daemon --timing-summary

echo ""
echo "🎉 All timing instrumentation tests passed!"
echo "✅ Timing data collection working"
echo "✅ Summary generation working"
echo "✅ CSV export working"
echo "✅ Performance comparison working"
echo ""
echo "💡 Note: Timing logs are only shown in benchmark mode (AKA_BENCHMARK=1)"
echo "💡 In normal operation, timing collection runs silently"

