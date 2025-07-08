# Home Directory Refactoring Design Plan

## Overview

This document outlines the comprehensive refactoring plan to replace the current `cache_dir: Option<PathBuf>` field in the `AKA` struct with a `home_dir: PathBuf` field. This change will centralize all path construction around a single home directory, making the codebase more testable and architecturally sound.

## Current Architecture Problems

### 1. Scattered Path Construction
Currently, paths are constructed in multiple places using different `dirs::` functions:
- `get_config_path()` uses `dirs::config_dir()` for `~/.config/aka/aka.yml`
- `setup_logging()` uses `dirs::data_local_dir()` for `~/.local/share/aka/logs/aka.log`
- `get_hash_cache_path()` uses `dirs::data_local_dir()` for `~/.local/share/aka/config.hash`
- `get_alias_cache_path_with_base()` uses `dirs::data_local_dir()` for `~/.local/share/aka/*.json`
- `determine_socket_path()` uses `dirs::home_dir()` for `~/.local/share/aka/socket`

### 2. Inconsistent Test Overrides
The current `cache_dir` field only overrides data directory paths, not config paths, leading to:
- Tests manually constructing config paths in temp directories
- Inconsistent path resolution between production and test environments
- Complex test setup with multiple temporary directories

### 3. Half-Measure Solution
The `cache_dir` field was a previous attempt to solve testability but only addressed part of the problem.

## Target Architecture

### 1. Single Source of Truth
All paths will be derived from a single `home_dir: PathBuf` field:
```rust
pub struct AKA {
    pub eol: bool,
    pub spec: Spec,
    pub config_hash: String,
    pub home_dir: PathBuf,  // Single source of truth
}
```

### 2. Centralized Path Construction
All path functions will accept `home_dir` parameter:
```rust
fn get_config_path(home_dir: &PathBuf) -> Result<PathBuf> {
    Ok(home_dir.join(".config").join("aka").join("aka.yml"))
}

fn get_data_dir(home_dir: &PathBuf) -> PathBuf {
    home_dir.join(".local").join("share").join("aka")
}

fn get_log_path(home_dir: &PathBuf) -> Result<PathBuf> {
    Ok(get_data_dir(home_dir).join("logs").join("aka.log"))
}
```

### 3. Clean Constructor
Single constructor that requires home directory:
```rust
impl AKA {
    pub fn new(eol: bool, home_dir: PathBuf) -> Result<Self> {
        let config_path = get_config_path(&home_dir)?;
        // ... rest of implementation
    }
}
```

## Implementation Plan

### Phase 1: Core Library Changes (`src/lib.rs`)

#### 1.1 Update AKA Struct
**Location:** `src/lib.rs:558-562`
```rust
// BEFORE:
pub struct AKA {
    pub eol: bool,
    pub spec: Spec,
    pub config_hash: String,
    pub cache_dir: Option<PathBuf>,
}

// AFTER:
pub struct AKA {
    pub eol: bool,
    pub spec: Spec,
    pub config_hash: String,
    pub home_dir: PathBuf,
}
```

#### 1.2 Replace Constructor Methods
**Location:** `src/lib.rs:565-570`
```rust
// REMOVE:
pub fn new(eol: bool, config: &Option<PathBuf>) -> Result<Self> {
    Self::new_with_cache_dir(eol, config, None)
}

pub fn new_with_cache_dir(eol: bool, config: &Option<PathBuf>, cache_dir: Option<&PathBuf>) -> Result<Self> {
    // ... existing implementation
}

// REPLACE WITH:
pub fn new(eol: bool, home_dir: PathBuf) -> Result<Self> {
    use std::time::Instant;

    let start_total = Instant::now();

    // Config path is always derived from home_dir
    let start_path = Instant::now();
    let config_path = get_config_path(&home_dir)?;
    let path_duration = start_path.elapsed();

    // Calculate config hash
    let config_hash = hash_config_file(&config_path)?;
    debug!("🔒 Config hash: {}", config_hash);

    // Time loader creation and config loading
    let start_load = Instant::now();
    let loader = Loader::new();
    let mut spec = loader.load(&config_path)?;
    let load_duration = start_load.elapsed();

    // Try to load from cache first
    let start_cache = Instant::now();
    if let Some(cached_aliases) = load_alias_cache(&config_hash, &home_dir)? {
        debug!("📋 Using cached aliases with usage counts");
        spec.aliases = cached_aliases;
    } else {
        debug!("📋 No cache found, initializing usage counts to 0");
        save_alias_cache(&config_hash, &spec.aliases, &home_dir)?;
    }
    let cache_duration = start_cache.elapsed();

    let total_duration = start_total.elapsed();
    debug!("🏗️  AKA::new() timing breakdown:");
    debug!("  📂 Path resolution: {:.3}ms", path_duration.as_secs_f64() * 1000.0);
    debug!("  📋 Config loading: {:.3}ms", load_duration.as_secs_f64() * 1000.0);
    debug!("  🗃️  Cache handling: {:.3}ms", cache_duration.as_secs_f64() * 1000.0);
    debug!("  🎯 Total AKA::new(): {:.3}ms", total_duration.as_secs_f64() * 1000.0);

    Ok(AKA { eol, spec, config_hash, home_dir })
}
```

#### 1.3 Update Path Functions

**1.3.1 Update `get_config_path()`**
**Location:** `src/lib.rs:298-312`
```rust
// BEFORE:
pub fn get_config_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Unable to determine config directory"))?;

    let config_path = config_dir.join("aka").join("aka.yml");

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    Ok(config_path)
}

// AFTER:
pub fn get_config_path(home_dir: &PathBuf) -> Result<PathBuf> {
    let config_path = home_dir.join(".config").join("aka").join("aka.yml");

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    Ok(config_path)
}
```

**1.3.2 Update `setup_logging()`**
**Location:** `src/lib.rs:320-348`
```rust
// BEFORE:
pub fn setup_logging() -> Result<()> {
    let log_dir = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("Unable to determine data directory"))?
        .join("aka")
        .join("logs");

    std::fs::create_dir_all(&log_dir)?;

    let log_file = log_dir.join("aka.log");
    // ... rest of implementation
}

// AFTER:
pub fn setup_logging(home_dir: &PathBuf) -> Result<()> {
    let log_dir = home_dir.join(".local").join("share").join("aka").join("logs");

    std::fs::create_dir_all(&log_dir)?;

    let log_file = log_dir.join("aka.log");
    // ... rest of implementation (unchanged)
}
```

**1.3.3 Update `get_hash_cache_path()`**
**Location:** `src/lib.rs:356-363`
```rust
// BEFORE:
pub fn get_hash_cache_path() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("Unable to determine data directory"))?;

    let cache_path = data_dir.join("aka").join("config.hash");
    Ok(cache_path)
}

// AFTER:
pub fn get_hash_cache_path(home_dir: &PathBuf) -> Result<PathBuf> {
    let cache_path = home_dir.join(".local").join("share").join("aka").join("config.hash");
    Ok(cache_path)
}
```

**1.3.4 Update `determine_socket_path()`**
**Location:** `src/lib.rs:785-797`
```rust
// BEFORE:
pub fn determine_socket_path() -> Result<PathBuf> {
    let home_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;

    let socket_dir = home_dir.join(".local").join("share").join("aka");
    std::fs::create_dir_all(&socket_dir)?;

    let socket_path = socket_dir.join("socket");
    Ok(socket_path)
}

// AFTER:
pub fn determine_socket_path(home_dir: &PathBuf) -> Result<PathBuf> {
    let socket_dir = home_dir.join(".local").join("share").join("aka");
    std::fs::create_dir_all(&socket_dir)?;

    let socket_path = socket_dir.join("socket");
    Ok(socket_path)
}
```

**1.3.5 Update `get_timing_file_path()`**
**Location:** `src/lib.rs:293-297`
```rust
// BEFORE:
pub fn get_timing_file_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Unable to determine config directory"))?;
    Ok(config_dir.join("aka").join("timing_data.csv"))
}

// AFTER:
pub fn get_timing_file_path(home_dir: &PathBuf) -> Result<PathBuf> {
    // Move timing data to share directory as requested
    Ok(home_dir.join(".local").join("share").join("aka").join("timing_data.csv"))
}
```

**1.3.6 Update Cache Functions**
**Location:** `src/lib.rs:800-873`
```rust
// BEFORE:
pub fn get_alias_cache_path_with_base(config_hash: &str, base_dir: Option<&PathBuf>) -> Result<PathBuf> {
    let data_dir = match base_dir {
        Some(dir) => dir.clone(),
        None => dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("Unable to determine data directory"))?
            .join("aka"),
    };

    let cache_path = data_dir.join(format!("{}.json", config_hash));
    Ok(cache_path)
}

// AFTER:
pub fn get_alias_cache_path(config_hash: &str, home_dir: &PathBuf) -> Result<PathBuf> {
    let data_dir = home_dir.join(".local").join("share").join("aka");
    let cache_path = data_dir.join(format!("{}.json", config_hash));
    Ok(cache_path)
}

// Update all related functions:
pub fn load_alias_cache(config_hash: &str, home_dir: &PathBuf) -> Result<Option<HashMap<String, Alias>>> {
    let cache_path = get_alias_cache_path(config_hash, home_dir)?;
    // ... rest unchanged
}

pub fn save_alias_cache(config_hash: &str, aliases: &HashMap<String, Alias>, home_dir: &PathBuf) -> Result<()> {
    let cache_path = get_alias_cache_path(config_hash, home_dir)?;
    // ... rest unchanged
}
```

**1.3.7 Update Hash Storage Functions**
**Location:** `src/lib.rs:365-379`
```rust
// BEFORE:
pub fn get_stored_hash() -> Result<Option<String>> {
    let hash_path = get_hash_cache_path()?;
    // ... rest of implementation
}

pub fn store_hash(hash: &str) -> Result<()> {
    let hash_path = get_hash_cache_path()?;
    // ... rest of implementation
}

// AFTER:
pub fn get_stored_hash(home_dir: &PathBuf) -> Result<Option<String>> {
    let hash_path = get_hash_cache_path(home_dir)?;
    // ... rest of implementation (unchanged)
}

pub fn store_hash(hash: &str, home_dir: &PathBuf) -> Result<()> {
    let hash_path = get_hash_cache_path(home_dir)?;
    // ... rest of implementation (unchanged)
}
```

**1.3.8 Update Health Check Function**
**Location:** `src/lib.rs:381-545`
```rust
// BEFORE:
pub fn execute_health_check(config: &Option<PathBuf>) -> Result<i32> {
    // ... existing implementation using get_config_path()
}

// AFTER:
pub fn execute_health_check(home_dir: &PathBuf) -> Result<i32> {
    let config_path = get_config_path(home_dir)?;

    // Check if config file exists
    if !config_path.exists() {
        debug!("🎯 Health check result: CONFIG_NOT_FOUND (returning 3)");
        return Ok(3); // Config file not found
    }

    // ... rest of implementation using config_path instead of resolving it
}
```

### Phase 2: Production Code Updates

#### 2.1 Update Main Binary (`src/bin/aka.rs`)

**2.1.1 Update AKA::new() Calls**
**Location:** `src/bin/aka.rs:976`
```rust
// BEFORE:
let mut aka = AKA::new(opts.eol, &opts.config)?;

// AFTER:
let home_dir = dirs::home_dir()
    .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
let mut aka = AKA::new(opts.eol, home_dir)?;
```

**2.1.2 Update setup_logging() Call**
**Location:** `src/bin/aka.rs:1108`
```rust
// BEFORE:
if let Err(e) = setup_logging() {
    eprintln!("Warning: Failed to set up logging: {}", e);
}

// AFTER:
let home_dir = dirs::home_dir()
    .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
if let Err(e) = setup_logging(&home_dir) {
    eprintln!("Warning: Failed to set up logging: {}", e);
}
```

**2.1.3 Update determine_socket_path() Calls**
**Locations:** Lines 52, 99, 504, 649, 831, 1071
```rust
// BEFORE:
let socket_path = determine_socket_path()?;

// AFTER:
let home_dir = dirs::home_dir()
    .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
let socket_path = determine_socket_path(&home_dir)?;
```

#### 2.2 Update Daemon Binary (`src/bin/aka-daemon.rs`)

**2.2.1 Update AKA::new() Calls**
**Location:** `src/bin/aka-daemon.rs:76, 78, 159, 337`
```rust
// BEFORE:
AKA::new_with_cache_dir(false, &Some(config_path.clone()), Some(&cache_dir_pathbuf))?
AKA::new(false, &Some(config_path.clone()))?
let mut new_aka = AKA::new(false, &Some(self.config_path.clone()))?;
match AKA::new(false, &Some(config_path_for_watcher.clone())) {

// AFTER:
let home_dir = dirs::home_dir()
    .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
AKA::new(false, home_dir.clone())?
AKA::new(false, home_dir.clone())?
let mut new_aka = AKA::new(false, home_dir.clone())?;
match AKA::new(false, home_dir.clone()) {
```

**2.2.2 Update Function Calls**
**Location:** `src/bin/aka-daemon.rs:500, 507`
```rust
// BEFORE:
if let Err(e) = setup_logging() {
    eprintln!("Warning: Failed to set up logging: {}", e);
}

let socket_path = match determine_socket_path() {

// AFTER:
let home_dir = dirs::home_dir()
    .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
if let Err(e) = setup_logging(&home_dir) {
    eprintln!("Warning: Failed to set up logging: {}", e);
}

let socket_path = match determine_socket_path(&home_dir) {
```

### Phase 3: Test Updates

#### 3.1 Architecture Validation Tests (`tests/architecture_validation.rs`)

**Current Pattern (4 instances):**
```rust
let config_temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
let cache_temp_dir = tempfile::tempdir().expect("Failed to create temp cache dir");
let config_file = config_temp_dir.path().join("aka.yml");
// ... write config file
let mut aka = AKA::new_with_cache_dir(false, &Some(config_file.clone()), Some(&cache_temp_dir.path().to_path_buf())).expect("Config should load");
```

**New Pattern:**
```rust
let temp_home = tempfile::tempdir().expect("Failed to create temp home dir");
let home_path = temp_home.path().to_path_buf();

// Create config directory structure
let config_dir = home_path.join(".config").join("aka");
std::fs::create_dir_all(&config_dir).expect("Failed to create config dir");
let config_file = config_dir.join("aka.yml");

// Create data directory structure
let data_dir = home_path.join(".local").join("share").join("aka");
std::fs::create_dir_all(&data_dir).expect("Failed to create data dir");

// Write config file
std::fs::write(&config_file, r#"
aliases:
  test_alias:
    value: "echo test"
"#).expect("Failed to write config file");

let mut aka = AKA::new(false, home_path).expect("Config should load");
```

#### 3.2 Usage Tracking Tests (`tests/usage_tracking_tests.rs`)

**Update Pattern (8 instances):**
Same pattern as architecture validation tests - replace the temp directory setup with proper home directory structure.

#### 3.3 File Watching Tests (`tests/file_watching_tests.rs`)

**Update Pattern (10 instances):**
Same pattern, plus update the `get_config_path()` test:

**Location:** `tests/file_watching_tests.rs:171-179`
```rust
// BEFORE:
fn test_get_config_path_function() {
    // Test that get_config_path function works
    let config_path = get_config_path();
    assert!(config_path.is_ok(), "get_config_path should succeed");

    let path = config_path.expect("get_config_path should return a valid path");
    assert!(path.to_string_lossy().contains("aka.yml"));
}

// AFTER:
fn test_get_config_path_function() {
    let temp_home = tempfile::tempdir().expect("Failed to create temp home dir");
    let home_path = temp_home.path().to_path_buf();

    let config_path = get_config_path(&home_path);
    assert!(config_path.is_ok(), "get_config_path should succeed");

    let path = config_path.expect("get_config_path should return a valid path");
    assert!(path.to_string_lossy().contains("aka.yml"));
    assert!(path.to_string_lossy().contains(".config/aka"));
}
```

### Phase 4: Documentation Updates

#### 4.1 Update Daemon Architecture Doc (`docs/daemon-architecture.md`)

**Location:** `docs/daemon-architecture.md:298`
```rust
// BEFORE:
let aka = AKA::new(eol, &None)?;

// AFTER:
let home_dir = dirs::home_dir()
    .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
let aka = AKA::new(eol, home_dir)?;
```

## Migration Strategy

### Step 1: Preparation
1. Run all existing tests to ensure baseline functionality
2. Create backup branch: `git checkout -b backup-before-home-dir-refactor`
3. Create feature branch: `git checkout -b feature/home-dir-refactor`

### Step 2: Core Library Changes
1. Update `AKA` struct definition
2. Replace constructor methods
3. Update all path utility functions
4. Update function signatures throughout the codebase
5. Run `cargo check` to identify compilation errors

### Step 3: Production Code Updates
1. Update `src/bin/aka.rs`
2. Update `src/bin/aka-daemon.rs`
3. Run `cargo check` again

### Step 4: Test Updates
1. Update all test files with new temp directory pattern
2. Run `cargo test` to ensure all tests pass
3. Verify test isolation (no shared state between tests)

### Step 5: Documentation and Cleanup
1. Update documentation files
2. Remove any unused functions or imports
3. Run `cargo clippy` for code quality checks
4. Run `cargo fmt` for consistent formatting

### Step 6: Validation
1. Run full test suite: `cargo test`
2. Test daemon functionality manually
3. Test with actual config files
4. Verify no regressions in path handling

## Expected Benefits

### 1. Improved Testability
- Single temp directory per test instead of multiple
- Consistent path resolution between production and test
- Easier test setup and teardown

### 2. Cleaner Architecture
- Single source of truth for all paths
- Centralized path construction logic
- Elimination of scattered `dirs::` calls

### 3. Better Error Handling
- Consistent error handling for path resolution
- Clear failure modes when home directory unavailable

### 4. XDG Compliance
- Maintains XDG Base Directory Specification compliance
- Proper directory structure: `~/.config/aka/`, `~/.local/share/aka/`

## Risk Mitigation

### 1. Backwards Compatibility
- No changes to config file format or location
- No changes to user-facing behavior
- All existing functionality preserved

### 2. Test Coverage
- All existing tests updated to use new pattern
- No reduction in test coverage
- Improved test isolation

### 3. Rollback Plan
- Backup branch created before changes
- Changes can be reverted if issues discovered
- No database migrations or persistent state changes

## Success Criteria

1. ✅ All existing tests pass with new implementation
2. ✅ No regressions in daemon functionality
3. ✅ Consistent path resolution in all environments
4. ✅ Simplified test setup code
5. ✅ Clean compiler output (no warnings)
6. ✅ Proper error handling for edge cases

## Implementation Checklist

### Core Library (`src/lib.rs`)
- [ ] Update `AKA` struct definition
- [ ] Replace constructor methods
- [ ] Update `get_config_path()`
- [ ] Update `setup_logging()`
- [ ] Update `get_hash_cache_path()`
- [ ] Update `determine_socket_path()`
- [ ] Update `get_timing_file_path()`
- [ ] Update cache functions
- [ ] Update hash storage functions
- [ ] Update health check function

### Production Code
- [ ] Update `src/bin/aka.rs` (7 locations)
- [ ] Update `src/bin/aka-daemon.rs` (6 locations)

### Tests
- [ ] Update `tests/architecture_validation.rs` (4 instances)
- [ ] Update `tests/usage_tracking_tests.rs` (8 instances)
- [ ] Update `tests/file_watching_tests.rs` (11 instances)

### Documentation
- [ ] Update `docs/daemon-architecture.md` (1 instance)

### Validation
- [ ] Run `cargo check`
- [ ] Run `cargo test`
- [ ] Run `cargo clippy`
- [ ] Manual testing of daemon functionality
- [ ] Verify path resolution in all scenarios

This refactoring will result in a cleaner, more testable, and more maintainable codebase while preserving all existing functionality.
