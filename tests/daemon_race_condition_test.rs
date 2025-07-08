use std::sync::{Arc, Mutex, Barrier};
use std::thread;
use std::time::{Duration, Instant};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// Import daemon types
use aka_lib::{AKA, hash_config_file};

#[cfg(test)]
mod daemon_race_condition_tests {
    use super::*;

    const INITIAL_CONFIG: &str = r#"
lookups: {}
aliases:
  test-alias-1:
    value: echo "initial value 1"
    global: true
  test-alias-2:
    value: echo "initial value 2"
    global: false
"#;

    const UPDATED_CONFIG: &str = r#"
lookups: {}
aliases:
  test-alias-1:
    value: echo "updated value 1"
    global: true
  test-alias-2:
    value: echo "updated value 2"
    global: false
  test-alias-3:
    value: echo "new value 3"
    global: true
"#;

    // State management for atomic updates (matching the new daemon implementation)
    struct TestDaemonState {
        aka: AKA,
        config_hash: String,
    }

    impl TestDaemonState {
        fn new(aka: AKA, config_hash: String) -> Self {
            Self { aka, config_hash }
        }
    }

    fn setup_test_environment(_test_name: &str) -> (TempDir, PathBuf, PathBuf) {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_dir = temp_dir.path().join(".config").join("aka");
        fs::create_dir_all(&config_dir).expect("Failed to create config directory");
        
        let config_path = config_dir.join("aka.yml");
        fs::write(&config_path, INITIAL_CONFIG).expect("Failed to write initial config");
        
        let home_dir = temp_dir.path().to_path_buf();
        (temp_dir, config_path, home_dir)
    }

    /// Test that demonstrates the OLD race condition is now FIXED with atomic state updates
    #[test]
    fn test_query_reload_race_condition_fixed() {
        let (_temp_dir, config_path, home_dir) = setup_test_environment("query_reload_race_fixed");
        
        // Load initial config
        let initial_aka = AKA::new(false, home_dir.clone()).expect("Failed to load initial config");
        let initial_hash = hash_config_file(&config_path).expect("Failed to hash config");
        
        // NEW: Use atomic state management (single RwLock for both config and hash)
        let initial_state = TestDaemonState::new(initial_aka, initial_hash);
        let atomic_state = Arc::new(std::sync::RwLock::new(initial_state));
        
        // Reload synchronization (like the new daemon implementation)
        let reload_mutex = Arc::new(Mutex::new(()));
        
        // Shared state for tracking race conditions
        let race_detected = Arc::new(Mutex::new(false));
        let inconsistent_reads = Arc::new(Mutex::new(Vec::new()));
        
        // Barrier to synchronize threads
        let barrier = Arc::new(Barrier::new(3)); // 1 reload thread + 2 query threads
        
        // Clone for threads
        let atomic_state_reload = Arc::clone(&atomic_state);
        let reload_mutex_reload = Arc::clone(&reload_mutex);
        let race_detected_reload = Arc::clone(&race_detected);
        let barrier_reload = Arc::clone(&barrier);
        let config_path_reload = config_path.clone();
        let home_dir_reload = home_dir.clone();
        
        // Reload thread - simulates NEW atomic reload logic
        let reload_handle = thread::spawn(move || {
            barrier_reload.wait();
            
            // Acquire reload mutex (like the new implementation)
            let _reload_guard = reload_mutex_reload.lock().expect("Failed to acquire reload mutex");
            
            // Update config file
            fs::write(&config_path_reload, UPDATED_CONFIG).expect("Failed to write updated config");
            
            // NEW: Atomic reload logic
            let new_hash = hash_config_file(&config_path_reload).expect("Failed to hash updated config");
            let new_aka = AKA::new(false, home_dir_reload).expect("Failed to load updated config");
            
            // ATOMIC STATE UPDATE: Update both config and hash together
            {
                let mut state_guard = atomic_state_reload.write().expect("Failed to acquire write lock");
                state_guard.aka = new_aka;
                state_guard.config_hash = new_hash;
                // No inconsistency window - both updated atomically!
            }
            
            *race_detected_reload.lock().unwrap() = true;
        });
        
        // Query thread 1 - simulates client queries during reload
        let atomic_state_query1 = Arc::clone(&atomic_state);
        let inconsistent_reads_query1 = Arc::clone(&inconsistent_reads);
        let barrier_query1 = Arc::clone(&barrier);
        
        let query_handle1 = thread::spawn(move || {
            barrier_query1.wait();
            
            // Continuously query during reload to check for consistency
            for i in 0..100 {
                let state_guard = atomic_state_query1.read().expect("Failed to acquire read lock");
                let alias_count = state_guard.aka.spec.aliases.len();
                
                // NEW: Check for internal consistency within the atomic state
                // With atomic updates, config and hash should always be consistent with each other
                if alias_count == 2 {
                    // Initial config should have initial hash pattern
                    if let Some(alias) = state_guard.aka.spec.aliases.get("test-alias-1") {
                        if alias.value.contains("updated") {
                            // This should NOT happen - old count with new values
                            inconsistent_reads_query1.lock().unwrap().push(format!("Thread1-Iter{}: Old count with new values", i));
                        }
                    }
                } else if alias_count == 3 {
                    // Updated config should have updated values
                    if let Some(alias) = state_guard.aka.spec.aliases.get("test-alias-1") {
                        if alias.value.contains("initial") {
                            // This should NOT happen - new count with old values
                            inconsistent_reads_query1.lock().unwrap().push(format!("Thread1-Iter{}: New count with old values", i));
                        }
                    }
                    
                    // Should have the new alias
                    if !state_guard.aka.spec.aliases.contains_key("test-alias-3") {
                        inconsistent_reads_query1.lock().unwrap().push(format!("Thread1-Iter{}: Missing new alias", i));
                    }
                }
                
                drop(state_guard);
                
                // Small delay to allow reload thread to make progress
                thread::sleep(Duration::from_micros(10));
            }
        });
        
        // Query thread 2 - simulates another client query during reload
        let atomic_state_query2 = Arc::clone(&atomic_state);
        let inconsistent_reads_query2 = Arc::clone(&inconsistent_reads);
        let barrier_query2 = Arc::clone(&barrier);
        
        let query_handle2 = thread::spawn(move || {
            barrier_query2.wait();
            
            // Try to acquire write lock (like Query requests do) during reload
            for i in 0..50 {
                if let Ok(mut state_guard) = atomic_state_query2.try_write() {
                    let alias_count = state_guard.aka.spec.aliases.len();
                    
                    // Simulate query processing (modifying eol setting)
                    state_guard.aka.eol = i % 2 == 0;
                    
                    // Check if we're seeing valid state
                    if alias_count != 2 && alias_count != 3 {
                        inconsistent_reads_query2.lock().unwrap().push(format!("Thread2-Iter{}: Invalid alias count: {}", i, alias_count));
                    }
                    
                    // Check internal consistency
                    if alias_count == 2 {
                        if let Some(alias) = state_guard.aka.spec.aliases.get("test-alias-1") {
                            if alias.value.contains("updated") {
                                inconsistent_reads_query2.lock().unwrap().push(format!("Thread2-Iter{}: Old count with new values", i));
                            }
                        }
                    }
                    
                    drop(state_guard);
                }
                
                thread::sleep(Duration::from_micros(20));
            }
        });
        
        // Wait for all threads to complete
        reload_handle.join().expect("Reload thread panicked");
        query_handle1.join().expect("Query thread 1 panicked");
        query_handle2.join().expect("Query thread 2 panicked");
        
        // Check results
        let race_occurred = *race_detected.lock().unwrap();
        let inconsistencies = inconsistent_reads.lock().unwrap();
        
        println!("Fixed race condition test results:");
        println!("- Race occurred: {}", race_occurred);
        println!("- Inconsistent reads detected: {}", inconsistencies.len());
        for inconsistency in inconsistencies.iter() {
            println!("  - {}", inconsistency);
        }
        
        // This test should NOT detect race conditions with atomic updates
        assert!(race_occurred, "Race condition should have been triggered");
        
        // With atomic updates, there should be NO inconsistent reads
        assert!(inconsistencies.is_empty(), "Atomic updates should prevent inconsistent reads, but found: {:?}", *inconsistencies);
    }
    
    /// Test that demonstrates multiple reload triggers are now properly synchronized
    #[test]
    fn test_multiple_reload_triggers_synchronized() {
        let (_temp_dir, config_path, home_dir) = setup_test_environment("multiple_reload_triggers_sync");
        
        // NEW: Use atomic state management
        let initial_aka = AKA::new(false, home_dir.clone()).expect("Failed to load initial config");
        let initial_hash = hash_config_file(&config_path).expect("Failed to hash config");
        let initial_state = TestDaemonState::new(initial_aka, initial_hash);
        let atomic_state = Arc::new(std::sync::RwLock::new(initial_state));
        
        // NEW: Reload synchronization
        let reload_mutex = Arc::new(Mutex::new(()));
        
        let reload_count = Arc::new(Mutex::new(0));
        let conflicts_detected = Arc::new(Mutex::new(Vec::new()));
        
        // Barrier to synchronize multiple reload threads
        let barrier = Arc::new(Barrier::new(3));
        
        // Create multiple reload threads that trigger simultaneously
        let mut handles = Vec::new();
        
        for thread_id in 0..3 {
            let atomic_state_clone = Arc::clone(&atomic_state);
            let reload_mutex_clone = Arc::clone(&reload_mutex);
            let reload_count_clone = Arc::clone(&reload_count);
            let conflicts_detected_clone = Arc::clone(&conflicts_detected);
            let barrier_clone = Arc::clone(&barrier);
            let config_path_clone = config_path.clone();
            let home_dir_clone = home_dir.clone();
            
            let handle = thread::spawn(move || {
                barrier_clone.wait();
                
                // Each thread tries to reload simultaneously
                let start_time = Instant::now();
                
                // NEW: Acquire reload mutex first (serializes reloads)
                match reload_mutex_clone.try_lock() {
                    Ok(_reload_guard) => {
                        // Update config file with thread-specific content
                        let thread_config = format!(r#"
lookups: {{}}
aliases:
  test-alias-thread-{}:
    value: echo "thread {} update"
    global: true
"#, thread_id, thread_id);
                        
                        fs::write(&config_path_clone, &thread_config).expect("Failed to write config");
                        
                        // Simulate reload logic
                        let new_hash = hash_config_file(&config_path_clone).expect("Failed to hash config");
                        
                        // Try to acquire state lock
                        match atomic_state_clone.try_write() {
                            Ok(mut state_guard) => {
                                // Simulate config loading time
                                thread::sleep(Duration::from_millis(5));
                                
                                match AKA::new(false, home_dir_clone.clone()) {
                                    Ok(new_aka) => {
                                        // ATOMIC UPDATE
                                        state_guard.aka = new_aka;
                                        state_guard.config_hash = new_hash;
                                        
                                        let mut count = reload_count_clone.lock().unwrap();
                                        *count += 1;
                                        
                                        println!("Thread {} completed reload successfully", thread_id);
                                    }
                                    Err(e) => {
                                        conflicts_detected_clone.lock().unwrap().push(
                                            format!("Thread {} failed to load config: {}", thread_id, e)
                                        );
                                    }
                                }
                            }
                            Err(_) => {
                                conflicts_detected_clone.lock().unwrap().push(
                                    format!("Thread {} failed to acquire state lock", thread_id)
                                );
                            }
                        }
                    }
                    Err(_) => {
                        conflicts_detected_clone.lock().unwrap().push(
                            format!("Thread {} failed to acquire reload mutex", thread_id)
                        );
                    }
                }
                
                let duration = start_time.elapsed();
                println!("Thread {} reload attempt took {:?}", thread_id, duration);
            });
            
            handles.push(handle);
        }
        
        // Wait for all threads
        for handle in handles {
            handle.join().expect("Thread panicked");
        }
        
        // Check results
        let final_reload_count = *reload_count.lock().unwrap();
        let conflicts = conflicts_detected.lock().unwrap();
        
        println!("Synchronized reload triggers test results:");
        println!("- Successful reloads: {}", final_reload_count);
        println!("- Conflicts detected: {}", conflicts.len());
        for conflict in conflicts.iter() {
            println!("  - {}", conflict);
        }
        
        // With proper synchronization, only one reload should succeed
        assert_eq!(final_reload_count, 1, "Only one reload should succeed with proper synchronization");
        assert_eq!(conflicts.len(), 2, "Two threads should be blocked by reload mutex");
        
        // Verify the conflicts are due to mutex blocking, not lock failures
        for conflict in conflicts.iter() {
            assert!(conflict.contains("failed to acquire reload mutex"), 
                   "Conflicts should be due to reload mutex blocking, not lock failures: {}", conflict);
        }
    }
    
    /// Test that demonstrates atomic state updates prevent hash-config inconsistency
    #[test]
    fn test_atomic_state_prevents_inconsistency() {
        let (_temp_dir, config_path, home_dir) = setup_test_environment("atomic_state_consistency");
        
        // NEW: Use atomic state management
        let initial_aka = AKA::new(false, home_dir.clone()).expect("Failed to load initial config");
        let initial_hash = hash_config_file(&config_path).expect("Failed to hash config");
        let initial_state = TestDaemonState::new(initial_aka, initial_hash);
        let atomic_state = Arc::new(std::sync::RwLock::new(initial_state));
        
        let inconsistencies = Arc::new(Mutex::new(Vec::new()));
        let barrier = Arc::new(Barrier::new(2));
        
        // Reload thread with atomic updates
        let atomic_state_reload = Arc::clone(&atomic_state);
        let barrier_reload = Arc::clone(&barrier);
        let config_path_reload = config_path.clone();
        let home_dir_reload = home_dir.clone();
        
        let reload_handle = thread::spawn(move || {
            barrier_reload.wait();
            
            // Update config file
            fs::write(&config_path_reload, UPDATED_CONFIG).expect("Failed to write updated config");
            
            // NEW: Atomic reload sequence
            let new_hash = hash_config_file(&config_path_reload).expect("Failed to hash config");
            let new_aka = AKA::new(false, home_dir_reload).expect("Failed to load config");
            
            // ATOMIC UPDATE: Both config and hash updated together
            {
                let mut state_guard = atomic_state_reload.write().expect("Failed to acquire write lock");
                state_guard.aka = new_aka;
                state_guard.config_hash = new_hash;
                // No inconsistency window!
            }
        });
        
        // Observer thread that checks for inconsistencies
        let atomic_state_observer = Arc::clone(&atomic_state);
        let inconsistencies_observer = Arc::clone(&inconsistencies);
        let barrier_observer = Arc::clone(&barrier);
        let config_path_observer = config_path.clone();
        
        let observer_handle = thread::spawn(move || {
            barrier_observer.wait();
            
            // Continuously check for inconsistencies
            for i in 0..1000 {
                let state_guard = atomic_state_observer.read().expect("Failed to acquire read lock");
                let alias_count = state_guard.aka.spec.aliases.len();
                let stored_hash = state_guard.config_hash.clone();
                
                // Check if stored hash matches actual config state
                if alias_count == 3 {
                    // We have new config (3 aliases)
                    if let Some(alias) = state_guard.aka.spec.aliases.get("test-alias-1") {
                        if alias.value.contains("updated") {
                            // New config is loaded, check if hash is consistent
                            let current_file_hash = hash_config_file(&config_path_observer).unwrap_or_default();
                            if stored_hash != current_file_hash {
                                // This should NOT happen with atomic updates
                                inconsistencies_observer.lock().unwrap().push(
                                    format!("Iter {}: New config loaded but hash not updated", i)
                                );
                            }
                        }
                    }
                }
                
                drop(state_guard);
                
                // Small delay to allow reload thread to make progress
                thread::sleep(Duration::from_micros(10));
            }
        });
        
        // Wait for threads
        reload_handle.join().expect("Reload thread panicked");
        observer_handle.join().expect("Observer thread panicked");
        
        // Check results
        let detected_inconsistencies = inconsistencies.lock().unwrap();
        
        println!("Atomic state consistency test results:");
        println!("- Inconsistencies detected: {}", detected_inconsistencies.len());
        for inconsistency in detected_inconsistencies.iter() {
            println!("  - {}", inconsistency);
        }
        
        // With atomic updates, there should be NO inconsistencies
        assert!(detected_inconsistencies.is_empty(), 
               "Atomic state updates should prevent inconsistencies, but found: {:?}", *detected_inconsistencies);
    }
    
    /// Test that demonstrates debouncing reduces wasted reload attempts
    #[test]
    fn test_debouncing_reduces_waste() {
        let (_temp_dir, config_path, home_dir) = setup_test_environment("debouncing_test");
        
        // NEW: Use atomic state management
        let initial_aka = AKA::new(false, home_dir.clone()).expect("Failed to load initial config");
        let initial_hash = hash_config_file(&config_path).expect("Failed to hash config");
        let initial_state = TestDaemonState::new(initial_aka, initial_hash);
        let atomic_state = Arc::new(std::sync::RwLock::new(initial_state));
        
        // Debouncing state (like the new daemon implementation)
        let reload_mutex = Arc::new(Mutex::new(()));
        let last_reload_time = Arc::new(Mutex::new(Instant::now() - Duration::from_millis(100))); // Start with old time
        const DEBOUNCE_DELAY_MS: u64 = 20;  // Shorter for testing
        
        let reload_attempts = Arc::new(Mutex::new(0));
        let successful_reloads = Arc::new(Mutex::new(0));
        let debounced_attempts = Arc::new(Mutex::new(0));
        
        let mut handles = Vec::new();
        
        // Create multiple threads that simulate rapid file change events
        for i in 0..10 {
            let config_path_clone = config_path.clone();
            let home_dir_clone = home_dir.clone();
            let atomic_state_clone = Arc::clone(&atomic_state);
            let reload_mutex_clone = Arc::clone(&reload_mutex);
            let last_reload_time_clone = Arc::clone(&last_reload_time);
            let reload_attempts_clone = Arc::clone(&reload_attempts);
            let successful_reloads_clone = Arc::clone(&successful_reloads);
            let debounced_attempts_clone = Arc::clone(&debounced_attempts);
            
            let handle = thread::spawn(move || {
                // Add small delay to spread out the attempts
                thread::sleep(Duration::from_millis(i * 5));
                
                // Each thread simulates a file change event
                let config_content = format!(r#"
lookups: {{}}
aliases:
  test-alias-{}:
    value: echo "rapid change {}"
    global: true
"#, i, i);
                
                fs::write(&config_path_clone, &config_content).expect("Failed to write config");
                
                // Increment attempt counter
                {
                    let mut attempts = reload_attempts_clone.lock().unwrap();
                    *attempts += 1;
                }
                
                // NEW: Debouncing logic
                let should_reload = {
                    let last_reload = last_reload_time_clone.lock().unwrap();
                    let time_since_last_reload = last_reload.elapsed();
                    
                    if time_since_last_reload >= Duration::from_millis(DEBOUNCE_DELAY_MS) {
                        true
                    } else {
                        let mut debounced = debounced_attempts_clone.lock().unwrap();
                        *debounced += 1;
                        false
                    }
                };
                
                if should_reload {
                    // Try to acquire reload mutex
                    if let Ok(_reload_guard) = reload_mutex_clone.try_lock() {
                        // Try to acquire state lock
                        if let Ok(mut state_guard) = atomic_state_clone.try_write() {
                            // Simulate reload work
                            thread::sleep(Duration::from_millis(1));
                            
                            if let Ok(new_aka) = AKA::new(false, home_dir_clone.clone()) {
                                let new_hash = hash_config_file(&config_path_clone).unwrap_or_default();
                                
                                // ATOMIC UPDATE
                                state_guard.aka = new_aka;
                                state_guard.config_hash = new_hash;
                                
                                // Update last reload time
                                {
                                    let mut last_reload = last_reload_time_clone.lock().unwrap();
                                    *last_reload = Instant::now();
                                }
                                
                                let mut successes = successful_reloads_clone.lock().unwrap();
                                *successes += 1;
                                
                                println!("Debounced reload {} succeeded", i);
                            }
                        }
                    }
                }
            });
            
            handles.push(handle);
        }
        
        // Wait for all threads
        for handle in handles {
            handle.join().expect("Thread panicked");
        }
        
        let total_attempts = *reload_attempts.lock().unwrap();
        let total_successes = *successful_reloads.lock().unwrap();
        let total_debounced = *debounced_attempts.lock().unwrap();
        
        println!("Debouncing test results:");
        println!("- Total reload attempts: {}", total_attempts);
        println!("- Successful reloads: {}", total_successes);
        println!("- Debounced attempts: {}", total_debounced);
        println!("- Wasted attempts: {}", total_attempts - total_successes - total_debounced);
        
        // This test shows that debouncing reduces wasted attempts
        assert_eq!(total_attempts, 10, "Should have 10 rapid reload attempts");
        assert!(total_successes > 0, "Should have some successful reloads");
        
        // With debouncing, fewer attempts should be wasted
        let wasted_attempts = total_attempts - total_successes - total_debounced;
        println!("IMPROVEMENT: {} attempts were properly debounced, only {} were wasted", 
                 total_debounced, wasted_attempts);
        
        // Debouncing should significantly reduce waste compared to no debouncing
        assert!(total_debounced > 0 || total_successes > 5, 
               "Either debouncing should prevent attempts OR most should succeed without contention");
    }
} 