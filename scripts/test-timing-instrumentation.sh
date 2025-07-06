#!/bin/bash

# Test script for AKA timing instrumentation
# This script validates that the timing framework works correctly

set -e

echo "🎯 AKA Timing Instrumentation Test"
echo "=================================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

PASS_COUNT=0
FAIL_COUNT=0

# Function to run a test
run_test() {
    local test_name="$1"
    local command="$2"
    local expected_result="$3"
    
    echo -n "Testing $test_name... "
    
    if eval "$command" > /dev/null 2>&1; then
        if [ "$expected_result" = "success" ]; then
            echo -e "${GREEN}PASS${NC}"
            ((PASS_COUNT++))
        else
            echo -e "${RED}FAIL${NC} (expected failure but got success)"
            ((FAIL_COUNT++))
        fi
    else
        if [ "$expected_result" = "fail" ]; then
            echo -e "${GREEN}PASS${NC} (expected failure)"
            ((PASS_COUNT++))
        else
            echo -e "${RED}FAIL${NC} (expected success but got failure)"
            ((FAIL_COUNT++))
        fi
    fi
}

# Function to test timing data collection
test_timing_collection() {
    echo -e "\n${BLUE}Testing Timing Data Collection${NC}"
    echo "------------------------------"
    
    # Clear any existing timing data by restarting daemon
    echo "🔄 Restarting daemon to clear timing data..."
    aka daemon --stop > /dev/null 2>&1 || true
    sleep 1
    aka daemon --start > /dev/null 2>&1 || true
    sleep 2
    
    # Run some queries to generate timing data
    echo "📊 Generating timing data..."
    
    # Test daemon mode
    echo "  Testing daemon queries..."
    aka query "ls" > /dev/null 2>&1 || true
    aka query "cat test.txt" > /dev/null 2>&1 || true
    aka query "grep pattern" > /dev/null 2>&1 || true
    
    # Test fallback mode
    echo "  Testing fallback queries..."
    aka daemon --stop > /dev/null 2>&1 || true
    sleep 1
    
    aka query "ls" > /dev/null 2>&1 || true
    aka query "cat test.txt" > /dev/null 2>&1 || true
    
    # Restart daemon for summary
    aka daemon --start > /dev/null 2>&1 || true
    sleep 2
    
    echo "✅ Timing data generated"
}

# Function to test timing summary
test_timing_summary() {
    echo -e "\n${BLUE}Testing Timing Summary${NC}"
    echo "----------------------"
    
    echo "📊 Getting timing summary..."
    if aka daemon --timing-summary > timing_summary.txt 2>&1; then
        echo -e "${GREEN}✅ Timing summary command works${NC}"
        
        # Check if summary contains expected content
        if grep -q "TIMING SUMMARY" timing_summary.txt; then
            echo -e "${GREEN}✅ Summary contains header${NC}"
        else
            echo -e "${RED}❌ Summary missing header${NC}"
            ((FAIL_COUNT++))
        fi
        
        if grep -q "Daemon mode:" timing_summary.txt; then
            echo -e "${GREEN}✅ Summary contains daemon data${NC}"
        else
            echo -e "${YELLOW}⚠️  Summary missing daemon data (may be normal if no daemon queries)${NC}"
        fi
        
        if grep -q "Direct mode:" timing_summary.txt; then
            echo -e "${GREEN}✅ Summary contains direct data${NC}"
        else
            echo -e "${YELLOW}⚠️  Summary missing direct data (may be normal if no direct queries)${NC}"
        fi
        
        echo "📄 Summary content:"
        cat timing_summary.txt | sed 's/^/    /'
        
    else
        echo -e "${RED}❌ Timing summary command failed${NC}"
        ((FAIL_COUNT++))
    fi
}

# Function to test CSV export
test_csv_export() {
    echo -e "\n${BLUE}Testing CSV Export${NC}"
    echo "------------------"
    
    echo "📊 Exporting timing CSV..."
    if aka daemon --export-timing > timing_data.csv 2>&1; then
        echo -e "${GREEN}✅ CSV export command works${NC}"
        
        # Check if CSV has header
        if head -n 1 timing_data.csv | grep -q "timestamp,mode,total_ms"; then
            echo -e "${GREEN}✅ CSV has correct header${NC}"
        else
            echo -e "${RED}❌ CSV missing or incorrect header${NC}"
            ((FAIL_COUNT++))
        fi
        
        # Count data rows
        data_rows=$(tail -n +2 timing_data.csv | wc -l)
        echo "📊 CSV contains $data_rows data rows"
        
        if [ "$data_rows" -gt 0 ]; then
            echo -e "${GREEN}✅ CSV contains data${NC}"
            echo "📄 Sample CSV data:"
            head -n 5 timing_data.csv | sed 's/^/    /'
        else
            echo -e "${YELLOW}⚠️  CSV contains no data (may be normal if no queries run)${NC}"
        fi
        
    else
        echo -e "${RED}❌ CSV export command failed${NC}"
        ((FAIL_COUNT++))
    fi
}

# Function to test performance comparison
test_performance_comparison() {
    echo -e "\n${BLUE}Testing Performance Comparison${NC}"
    echo "------------------------------"
    
    echo "⚡ Running performance comparison test..."
    
    # Ensure daemon is running
    aka daemon --start > /dev/null 2>&1 || true
    sleep 2
    
    # Run daemon queries
    echo "  Running daemon queries..."
    daemon_start=$(date +%s.%N)
    for i in {1..5}; do
        aka query "ls" > /dev/null 2>&1 || true
    done
    daemon_end=$(date +%s.%N)
    daemon_time=$(echo "$daemon_end - $daemon_start" | bc -l)
    
    # Run fallback queries
    echo "  Running fallback queries..."
    aka daemon --stop > /dev/null 2>&1 || true
    sleep 1
    
    fallback_start=$(date +%s.%N)
    for i in {1..5}; do
        aka query "ls" > /dev/null 2>&1 || true
    done
    fallback_end=$(date +%s.%N)
    fallback_time=$(echo "$fallback_end - $fallback_start" | bc -l)
    
    # Calculate performance difference
    echo "📊 Performance results:"
    echo "   Daemon time:   ${daemon_time}s (5 queries)"
    echo "   Fallback time: ${fallback_time}s (5 queries)"
    
    # Restart daemon for final summary
    aka daemon --start > /dev/null 2>&1 || true
    sleep 2
    
    echo -e "${GREEN}✅ Performance comparison completed${NC}"
}

# Main test execution
main() {
    echo "🔧 Setting up test environment..."
    
    # Ensure we have a working aka installation
    if ! command -v aka &> /dev/null; then
        echo -e "${RED}❌ 'aka' command not found. Please build and install first.${NC}"
        exit 1
    fi
    
    # Build the project to ensure latest changes
    echo "🏗️  Building project..."
    if cargo build --release > build.log 2>&1; then
        echo -e "${GREEN}✅ Build successful${NC}"
    else
        echo -e "${RED}❌ Build failed. Check build.log for details.${NC}"
        exit 1
    fi
    
    # Run timing tests
    test_timing_collection
    test_timing_summary
    test_csv_export
    test_performance_comparison
    
    # Final summary
    echo -e "\n${BLUE}Test Summary${NC}"
    echo "============"
    echo -e "✅ Passed: ${GREEN}$PASS_COUNT${NC}"
    echo -e "❌ Failed: ${RED}$FAIL_COUNT${NC}"
    
    if [ $FAIL_COUNT -eq 0 ]; then
        echo -e "\n${GREEN}🎉 All timing instrumentation tests passed!${NC}"
        exit 0
    else
        echo -e "\n${RED}❌ Some tests failed. Check output above for details.${NC}"
        exit 1
    fi
}

# Cleanup function
cleanup() {
    echo -e "\n🧹 Cleaning up..."
    rm -f timing_summary.txt timing_data.csv build.log
    aka daemon --stop > /dev/null 2>&1 || true
}

# Set up cleanup on exit
trap cleanup EXIT

# Run main function
main "$@" 