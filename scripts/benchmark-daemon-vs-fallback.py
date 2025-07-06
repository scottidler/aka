#!/usr/bin/env python3

"""
Benchmark script to compare daemon vs fallback performance in AKA.
This script runs multiple iterations of both modes and provides statistical analysis.
"""

import os
import sys
import time
import subprocess
import statistics
import json
from pathlib import Path
from typing import List, Dict, Tuple

# Enable benchmark mode for detailed timing
os.environ['AKA_BENCHMARK'] = '1'
os.environ['RUST_LOG'] = 'info'

class AKABenchmark:
    def __init__(self, iterations: int = 10):
        self.iterations = iterations
        self.aka_binary = self._find_aka_binary()
        self.results = {
            'daemon': [],
            'direct': [],
            'wall_clock_daemon': [],
            'wall_clock_direct': []
        }

    def _find_aka_binary(self) -> str:
        """Find the AKA binary path."""
        # Try common locations
        candidates = [
            "./target/release/aka",
            "./target/debug/aka",
            "aka"  # System PATH
        ]

        for candidate in candidates:
            try:
                result = subprocess.run([candidate, "--version"],
                                      capture_output=True, text=True, timeout=5)
                if result.returncode == 0:
                    return candidate
            except (subprocess.TimeoutExpired, FileNotFoundError):
                continue

        raise RuntimeError("Could not find AKA binary. Please build the project first.")

    def _run_command(self, cmd: List[str], timeout: int = 10) -> Tuple[bool, str, float]:
        """Run a command and return success, output, and wall-clock time."""
        start_time = time.time()
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
            end_time = time.time()
            return result.returncode == 0, result.stdout + result.stderr, end_time - start_time
        except subprocess.TimeoutExpired:
            end_time = time.time()
            return False, "Command timed out", end_time - start_time

    def _start_daemon(self) -> bool:
        """Start the AKA daemon."""
        print("🚀 Starting daemon...")
        success, output, _ = self._run_command([self.aka_binary, "daemon", "--start"])
        if success:
            time.sleep(2)  # Give daemon time to start
            return True
        else:
            print(f"❌ Failed to start daemon: {output}")
            return False

    def _stop_daemon(self) -> bool:
        """Stop the AKA daemon."""
        print("🛑 Stopping daemon...")
        success, output, _ = self._run_command([self.aka_binary, "daemon", "--stop"])
        if success:
            time.sleep(1)  # Give daemon time to stop
            return True
        else:
            print(f"⚠️  Failed to stop daemon cleanly: {output}")
            return False

    def _is_daemon_running(self) -> bool:
        """Check if daemon is running."""
        success, output, _ = self._run_command([self.aka_binary, "daemon", "--status"])
        return success and "running" in output.lower()

    def _run_query(self, query: str, mode: str) -> Tuple[bool, float]:
        """Run a single query and return success and wall-clock time."""
        success, output, wall_time = self._run_command([self.aka_binary, "query", query])
        return success, wall_time

    def _extract_timing_from_logs(self, log_output: str) -> float:
        """Extract timing data from log output (in benchmark mode)."""
        # Look for timing breakdown in logs
        lines = log_output.split('\n')
        for line in lines:
            if "Total execution:" in line and "ms" in line:
                # Extract the timing value
                parts = line.split("Total execution:")
                if len(parts) > 1:
                    timing_part = parts[1].strip()
                    # Extract number before "ms"
                    if "ms" in timing_part:
                        try:
                            timing_str = timing_part.split("ms")[0].strip()
                            return float(timing_str)
                        except ValueError:
                            continue
        return 0.0

    def benchmark_daemon_mode(self) -> None:
        """Benchmark daemon mode performance."""
        print(f"👹 Benchmarking daemon mode ({self.iterations} iterations)...")

        if not self._start_daemon():
            raise RuntimeError("Failed to start daemon for benchmarking")

        try:
            for i in range(self.iterations):
                print(f"  Running daemon query {i+1}/{self.iterations}...")
                success, wall_time = self._run_query("ls -la", "daemon")

                if success:
                    self.results['wall_clock_daemon'].append(wall_time * 1000)  # Convert to ms
                    print(f"    Wall-clock time: {wall_time*1000:.2f}ms")
                else:
                    print(f"    ❌ Query {i+1} failed")

                time.sleep(0.1)  # Small delay between queries

        finally:
            self._stop_daemon()

    def benchmark_direct_mode(self) -> None:
        """Benchmark direct mode performance."""
        print(f"📥 Benchmarking direct mode ({self.iterations} iterations)...")

        # Ensure daemon is stopped
        if self._is_daemon_running():
            self._stop_daemon()

        for i in range(self.iterations):
            print(f"  Running direct query {i+1}/{self.iterations}...")
            success, wall_time = self._run_query("ls -la", "direct")

            if success:
                self.results['wall_clock_direct'].append(wall_time * 1000)  # Convert to ms
                print(f"    Wall-clock time: {wall_time*1000:.2f}ms")
            else:
                print(f"    ❌ Query {i+1} failed")

            time.sleep(0.1)  # Small delay between queries

    def _get_timing_summary(self) -> Dict:
        """Get timing summary from AKA."""
        print("📊 Collecting timing summary...")

        # Start daemon to access timing summary
        if not self._is_daemon_running():
            self._start_daemon()

        try:
            success, output, _ = self._run_command([self.aka_binary, "daemon", "--timing-summary"])
            if success:
                return self._parse_timing_summary(output)
            else:
                print(f"❌ Failed to get timing summary: {output}")
                return {}
        finally:
            self._stop_daemon()

    def _parse_timing_summary(self, output: str) -> Dict:
        """Parse timing summary output."""
        results = {}
        lines = output.split('\n')

        for line in lines:
            line = line.strip()
            if "Daemon mode:" in line:
                # Extract daemon timing
                parts = line.split()
                for i, part in enumerate(parts):
                    if part.endswith("ms") and i > 0:
                        try:
                            results['daemon_avg_ms'] = float(part.replace('ms', ''))
                            break
                        except ValueError:
                            continue
            elif "Direct mode:" in line:
                # Extract direct timing
                parts = line.split()
                for i, part in enumerate(parts):
                    if part.endswith("ms") and i > 0:
                        try:
                            results['direct_avg_ms'] = float(part.replace('ms', ''))
                            break
                        except ValueError:
                            continue
            elif "Samples:" in line:
                # Extract sample count
                parts = line.split()
                for i, part in enumerate(parts):
                    if part.isdigit():
                        if 'daemon_samples' not in results:
                            results['daemon_samples'] = int(part)
                        else:
                            results['direct_samples'] = int(part)
                        break

        return results

    def generate_report(self) -> None:
        """Generate a comprehensive performance report."""
        print("\n" + "="*60)
        print("🎯 AKA DAEMON VS FALLBACK PERFORMANCE REPORT")
        print("="*60)

        # Get internal timing data
        timing_summary = self._get_timing_summary()

        # Wall-clock statistics
        if self.results['wall_clock_daemon']:
            daemon_wall_avg = statistics.mean(self.results['wall_clock_daemon'])
            daemon_wall_std = statistics.stdev(self.results['wall_clock_daemon']) if len(self.results['wall_clock_daemon']) > 1 else 0
            daemon_wall_min = min(self.results['wall_clock_daemon'])
            daemon_wall_max = max(self.results['wall_clock_daemon'])
        else:
            daemon_wall_avg = daemon_wall_std = daemon_wall_min = daemon_wall_max = 0

        if self.results['wall_clock_direct']:
            direct_wall_avg = statistics.mean(self.results['wall_clock_direct'])
            direct_wall_std = statistics.stdev(self.results['wall_clock_direct']) if len(self.results['wall_clock_direct']) > 1 else 0
            direct_wall_min = min(self.results['wall_clock_direct'])
            direct_wall_max = max(self.results['wall_clock_direct'])
        else:
            direct_wall_avg = direct_wall_std = direct_wall_min = direct_wall_max = 0

        # Wall-clock performance comparison
        print("\n📊 WALL-CLOCK PERFORMANCE (Process Startup + Processing)")
        print("-" * 50)
        print(f"👹 Daemon Mode:")
        print(f"   Average: {daemon_wall_avg:.1f}ms")
        print(f"   Std Dev: {daemon_wall_std:.1f}ms")
        print(f"   Range:   {daemon_wall_min:.1f}ms - {daemon_wall_max:.1f}ms")
        print(f"   Samples: {len(self.results['wall_clock_daemon'])}")

        print(f"\n📥 Direct Mode:")
        print(f"   Average: {direct_wall_avg:.1f}ms")
        print(f"   Std Dev: {direct_wall_std:.1f}ms")
        print(f"   Range:   {direct_wall_min:.1f}ms - {direct_wall_max:.1f}ms")
        print(f"   Samples: {len(self.results['wall_clock_direct'])}")

        if daemon_wall_avg > 0 and direct_wall_avg > 0:
            wall_improvement = direct_wall_avg - daemon_wall_avg
            wall_improvement_pct = (wall_improvement / direct_wall_avg) * 100
            print(f"\n⚡ Wall-Clock Improvement:")
            print(f"   Daemon is {wall_improvement:.1f}ms faster ({wall_improvement_pct:.1f}% improvement)")

        # Internal timing comparison (if available)
        if timing_summary:
            print("\n🔍 INTERNAL PROCESSING PERFORMANCE (Config + Processing Only)")
            print("-" * 50)

            daemon_internal = timing_summary.get('daemon_avg_ms', 0)
            direct_internal = timing_summary.get('direct_avg_ms', 0)
            daemon_samples = timing_summary.get('daemon_samples', 0)
            direct_samples = timing_summary.get('direct_samples', 0)

            print(f"👹 Daemon Mode:")
            print(f"   Average: {daemon_internal:.3f}ms")
            print(f"   Samples: {daemon_samples}")

            print(f"\n📥 Direct Mode:")
            print(f"   Average: {direct_internal:.3f}ms")
            print(f"   Samples: {direct_samples}")

            if daemon_internal > 0 and direct_internal > 0:
                internal_improvement = direct_internal - daemon_internal
                internal_improvement_pct = (internal_improvement / direct_internal) * 100
                print(f"\n⚡ Internal Processing Improvement:")
                print(f"   Daemon is {internal_improvement:.3f}ms faster ({internal_improvement_pct:.1f}% improvement)")

        # Performance analysis
        print("\n🎯 PERFORMANCE ANALYSIS")
        print("-" * 50)

        if daemon_wall_avg > 0 and direct_wall_avg > 0:
            startup_overhead = daemon_wall_avg - timing_summary.get('daemon_avg_ms', 0)
            print(f"Process startup overhead: ~{startup_overhead:.1f}ms ({startup_overhead/daemon_wall_avg*100:.1f}% of total)")

            config_overhead = timing_summary.get('direct_avg_ms', 0) - timing_summary.get('daemon_avg_ms', 0)
            print(f"Config loading overhead: ~{config_overhead:.3f}ms (eliminated by daemon)")

            print(f"\nKey Insights:")
            print(f"• Process startup dominates wall-clock time ({startup_overhead/daemon_wall_avg*100:.1f}%)")
            print(f"• Config loading is the main internal bottleneck")
            print(f"• Daemon eliminates config loading entirely")
            print(f"• Performance benefit scales with usage frequency")

        # Usage recommendations
        print("\n💡 USAGE RECOMMENDATIONS")
        print("-" * 50)
        print("• Interactive CLI: Minimal wall-clock benefit, but cleaner logs")
        print("• High-frequency usage: Daemon provides significant cumulative savings")
        print("• ZLE integration: Daemon essential for responsive shell experience")
        print("• Batch scripts: Linear scaling makes daemon crucial for performance")

        # Export detailed data
        self._export_detailed_data()

        print("\n" + "="*60)
        print("🎉 Benchmark completed successfully!")
        print("="*60)

    def _export_detailed_data(self) -> None:
        """Export detailed benchmark data to JSON."""
        data = {
            'benchmark_config': {
                'iterations': self.iterations,
                'timestamp': time.time(),
                'aka_binary': self.aka_binary
            },
            'results': self.results,
            'summary': self._get_timing_summary()
        }

        output_file = Path("benchmark_results.json")
        with open(output_file, 'w') as f:
            json.dump(data, f, indent=2)

        print(f"\n📄 Detailed data exported to: {output_file}")

    def run(self) -> None:
        """Run the complete benchmark suite."""
        print("🚀 Starting AKA Daemon vs Fallback Benchmark")
        print(f"Binary: {self.aka_binary}")
        print(f"Iterations: {self.iterations}")
        print(f"Benchmark mode: {'✅ ENABLED' if os.environ.get('AKA_BENCHMARK') else '❌ DISABLED'}")
        print()

        try:
            # Clean up any existing daemon
            if self._is_daemon_running():
                self._stop_daemon()

            # Run benchmarks
            self.benchmark_daemon_mode()
            self.benchmark_direct_mode()

            # Generate report
            self.generate_report()

        except KeyboardInterrupt:
            print("\n❌ Benchmark interrupted by user")
            self._stop_daemon()
            sys.exit(1)
        except Exception as e:
            print(f"\n❌ Benchmark failed: {e}")
            self._stop_daemon()
            sys.exit(1)

def main():
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(description="Benchmark AKA daemon vs fallback performance")
    parser.add_argument("--iterations", "-i", type=int, default=10,
                       help="Number of iterations to run (default: 10)")
    parser.add_argument("--quick", "-q", action="store_true",
                       help="Quick test with 3 iterations")

    args = parser.parse_args()

    iterations = 3 if args.quick else args.iterations

    benchmark = AKABenchmark(iterations=iterations)
    benchmark.run()

if __name__ == "__main__":
    main()
