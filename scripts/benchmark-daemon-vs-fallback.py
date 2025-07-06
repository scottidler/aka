#!/usr/bin/env python3
"""
Benchmark script to measure daemon vs fallback performance in the AKA alias system.

This script tests both execution paths:
1. Daemon mode: Uses persistent aka-daemon process
2. Fallback mode: Direct config loading (process-per-query)

The script uses the built-in timing instrumentation to get exact measurements.
"""

import subprocess
import time
import json
import csv
import sys
import os
from pathlib import Path
from typing import Dict, List, Tuple, Optional

class AkaBenchmark:
    def __init__(self, config_file: Optional[str] = None):
        self.config_file = config_file
        self.daemon_running = False
        
    def ensure_daemon_stopped(self):
        """Ensure daemon is stopped for fallback testing."""
        try:
            subprocess.run(['aka', 'daemon', '--stop'], 
                         capture_output=True, timeout=10)
            time.sleep(1)  # Give daemon time to stop
            self.daemon_running = False
            print("🛑 Daemon stopped")
        except Exception as e:
            print(f"⚠️  Warning: Could not stop daemon: {e}")
    
    def ensure_daemon_running(self):
        """Ensure daemon is running for daemon testing."""
        if self.daemon_running:
            return
            
        try:
            # Check if daemon is already running
            result = subprocess.run(['aka', 'daemon', '--status'], 
                                  capture_output=True, timeout=10)
            if result.returncode == 0:
                self.daemon_running = True
                print("✅ Daemon already running")
                return
                
            # Start daemon
            subprocess.run(['aka', 'daemon', '--start'], 
                         capture_output=True, timeout=10)
            time.sleep(2)  # Give daemon time to start
            
            # Verify daemon started
            result = subprocess.run(['aka', 'daemon', '--status'], 
                                  capture_output=True, timeout=10)
            if result.returncode == 0:
                self.daemon_running = True
                print("🚀 Daemon started successfully")
            else:
                raise Exception("Daemon failed to start")
                
        except Exception as e:
            print(f"❌ Error starting daemon: {e}")
            raise
    
    def run_query(self, query: str, mode: str) -> Dict:
        """Run a single query and return timing data."""
        cmd = ['aka', 'query', query]
        if self.config_file:
            cmd.extend(['-c', self.config_file])
            
        try:
            start_time = time.perf_counter()
            result = subprocess.run(cmd, capture_output=True, 
                                  timeout=30, text=True)
            end_time = time.perf_counter()
            
            wall_time = (end_time - start_time) * 1000  # Convert to ms
            
            return {
                'mode': mode,
                'query': query,
                'wall_time_ms': wall_time,
                'returncode': result.returncode,
                'stdout': result.stdout.strip(),
                'stderr': result.stderr.strip(),
                'success': result.returncode == 0
            }
        except subprocess.TimeoutExpired:
            return {
                'mode': mode,
                'query': query,
                'wall_time_ms': 30000,  # Timeout
                'returncode': -1,
                'stdout': '',
                'stderr': 'TIMEOUT',
                'success': False
            }
        except Exception as e:
            return {
                'mode': mode,
                'query': query,
                'wall_time_ms': 0,
                'returncode': -1,
                'stdout': '',
                'stderr': str(e),
                'success': False
            }
    
    def get_timing_summary(self) -> Optional[Dict]:
        """Get timing summary from aka daemon."""
        try:
            result = subprocess.run(['aka', 'daemon', '--timing-summary'], 
                                  capture_output=True, timeout=10, text=True)
            if result.returncode == 0:
                # Parse the output for timing data
                lines = result.stdout.strip().split('\n')
                summary = {}
                
                for line in lines:
                    if 'Average:' in line and 'ms' in line:
                        if 'Daemon mode:' in lines[lines.index(line) - 1]:
                            # Extract daemon average
                            ms_value = line.split('Average:')[1].split('ms')[0].strip()
                            summary['daemon_avg_ms'] = float(ms_value)
                        elif 'Direct mode:' in lines[lines.index(line) - 1]:
                            # Extract direct average
                            ms_value = line.split('Average:')[1].split('ms')[0].strip()
                            summary['direct_avg_ms'] = float(ms_value)
                    elif 'Samples:' in line:
                        samples = int(line.split('Samples:')[1].strip())
                        if 'daemon_samples' not in summary:
                            summary['daemon_samples'] = samples
                        else:
                            summary['direct_samples'] = samples
                    elif 'Daemon is' in line and 'faster' in line:
                        # Extract performance improvement
                        parts = line.split()
                        improvement_ms = float(parts[2].replace('ms', ''))
                        percentage = float(parts[4].replace('(', '').replace('%', ''))
                        summary['improvement_ms'] = improvement_ms
                        summary['improvement_percent'] = percentage
                
                return summary
        except Exception as e:
            print(f"⚠️  Could not get timing summary: {e}")
        return None
    
    def export_timing_csv(self) -> Optional[str]:
        """Export detailed timing data as CSV."""
        try:
            result = subprocess.run(['aka', 'daemon', '--export-timing'], 
                                  capture_output=True, timeout=10, text=True)
            if result.returncode == 0:
                return result.stdout.strip()
        except Exception as e:
            print(f"⚠️  Could not export timing CSV: {e}")
        return None
    
    def run_benchmark(self, queries: List[str], iterations: int = 10) -> Dict:
        """Run comprehensive benchmark comparing daemon vs fallback."""
        results = {
            'daemon_results': [],
            'fallback_results': [],
            'summary': {}
        }
        
        print(f"🎯 Running benchmark with {len(queries)} queries, {iterations} iterations each")
        print(f"📋 Queries: {queries}")
        
        # Test daemon mode
        print("\n👹 Testing DAEMON mode...")
        self.ensure_daemon_running()
        
        for i in range(iterations):
            print(f"  Iteration {i+1}/{iterations}")
            for query in queries:
                result = self.run_query(query, 'daemon')
                results['daemon_results'].append(result)
                if not result['success']:
                    print(f"    ⚠️  Query failed: {query} -> {result['stderr']}")
                else:
                    print(f"    ✅ {query} -> {result['wall_time_ms']:.1f}ms")
        
        # Test fallback mode  
        print("\n📥 Testing FALLBACK mode...")
        self.ensure_daemon_stopped()
        
        for i in range(iterations):
            print(f"  Iteration {i+1}/{iterations}")
            for query in queries:
                result = self.run_query(query, 'fallback')
                results['fallback_results'].append(result)
                if not result['success']:
                    print(f"    ⚠️  Query failed: {query} -> {result['stderr']}")
                else:
                    print(f"    ✅ {query} -> {result['wall_time_ms']:.1f}ms")
        
        # Restart daemon for summary collection
        print("\n📊 Collecting internal timing data...")
        self.ensure_daemon_running()
        
        # Get timing summary from internal instrumentation
        timing_summary = self.get_timing_summary()
        if timing_summary:
            results['summary']['internal_timing'] = timing_summary
        
        # Calculate wall-clock averages
        daemon_times = [r['wall_time_ms'] for r in results['daemon_results'] if r['success']]
        fallback_times = [r['wall_time_ms'] for r in results['fallback_results'] if r['success']]
        
        if daemon_times and fallback_times:
            daemon_avg = sum(daemon_times) / len(daemon_times)
            fallback_avg = sum(fallback_times) / len(fallback_times)
            improvement = fallback_avg - daemon_avg
            improvement_percent = (improvement / fallback_avg) * 100
            
            results['summary']['wall_clock'] = {
                'daemon_avg_ms': daemon_avg,
                'fallback_avg_ms': fallback_avg,
                'daemon_samples': len(daemon_times),
                'fallback_samples': len(fallback_times),
                'improvement_ms': improvement,
                'improvement_percent': improvement_percent
            }
        
        return results
    
    def print_results(self, results: Dict):
        """Print benchmark results in a nice format."""
        print("\n" + "="*60)
        print("📊 BENCHMARK RESULTS")
        print("="*60)
        
        # Wall clock timing
        if 'wall_clock' in results['summary']:
            wc = results['summary']['wall_clock']
            print(f"\n🕐 Wall Clock Timing:")
            print(f"   👹 Daemon avg:   {wc['daemon_avg_ms']:.3f}ms ({wc['daemon_samples']} samples)")
            print(f"   📥 Fallback avg: {wc['fallback_avg_ms']:.3f}ms ({wc['fallback_samples']} samples)")
            print(f"   ⚡ Improvement:  {wc['improvement_ms']:.3f}ms ({wc['improvement_percent']:.1f}%)")
        
        # Internal timing (from instrumentation)
        if 'internal_timing' in results['summary']:
            it = results['summary']['internal_timing']
            print(f"\n🔍 Internal Timing (from instrumentation):")
            if 'daemon_avg_ms' in it:
                print(f"   👹 Daemon avg:   {it['daemon_avg_ms']:.3f}ms ({it.get('daemon_samples', 0)} samples)")
            if 'direct_avg_ms' in it:
                print(f"   📥 Direct avg:   {it['direct_avg_ms']:.3f}ms ({it.get('direct_samples', 0)} samples)")
            if 'improvement_ms' in it:
                print(f"   ⚡ Improvement:  {it['improvement_ms']:.3f}ms ({it['improvement_percent']:.1f}%)")
        
        # Export detailed CSV if available
        csv_data = self.export_timing_csv()
        if csv_data:
            timestamp = int(time.time())
            csv_filename = f"aka_timing_data_{timestamp}.csv"
            with open(csv_filename, 'w') as f:
                f.write(csv_data)
            print(f"\n📄 Detailed timing data exported to: {csv_filename}")

def main():
    """Main benchmark execution."""
    import argparse
    
    parser = argparse.ArgumentParser(description='Benchmark AKA daemon vs fallback performance')
    parser.add_argument('-c', '--config', help='Config file to use')
    parser.add_argument('-i', '--iterations', type=int, default=10, 
                       help='Number of iterations per test (default: 10)')
    parser.add_argument('-q', '--queries', nargs='+', 
                       default=['ls', 'cat test.txt', 'grep pattern file.txt'],
                       help='Queries to test (default: ls, cat test.txt, grep pattern file.txt)')
    
    args = parser.parse_args()
    
    print("🚀 AKA Daemon vs Fallback Benchmark")
    print("=" * 40)
    
    benchmark = AkaBenchmark(config_file=args.config)
    results = benchmark.run_benchmark(args.queries, args.iterations)
    benchmark.print_results(results)

if __name__ == '__main__':
    main() 