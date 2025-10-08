# Alias Caching Investigation - Complete Analysis

## Date
2025-10-08

## Executive Summary

**Initial Report:** User reported that newly added aliases don't work and redefined aliases return stale values.

**Investigation Result:** After deep investigation with detailed tracing, we discovered:

1. ✅ **The caching system works correctly** - merge logic, parser, and sync all function as designed
2. ✅ **The test that "failed" was exposing correct behavior** - using an alias increments its count
3. ❌ **The real bug: daemon cache staleness** - file watcher doesn't reliably trigger reload
4. ❌ **Poor UX: users don't know cache is stale** - sourcing .zshrc doesn't help, no warnings

**Root Cause:** The daemon holds a cache in memory and doesn't always detect config changes. When users modify config and source `.zshrc`, they expect changes to take effect, but the daemon continues serving stale values.

**Fix Plan:**
- Phase 1: Fix test expectations (test was wrong, not code)
- Phase 2: Add proactive cache validation in daemon (check hash on every query)
- Phase 3: Improve file watcher with polling fallback
- Phase 4: Better user communication about cache state
- Phase 5: Quick user fix: `aka daemon --reload`

## Bug Report

### User's Symptoms

1. Added "desk" as additional alias to existing "home" entry: `home|desk: ssh desk.lan`
   - The "desk" alias doesn't work with the daemon
   - Only the original "home" alias works

2. Redefined "lappy" from standalone alias to part of `work|lappy: ssh ltl-7007.lan`
   - Returns old cached value "ltl-7007" instead of new value "ssh ltl-7007.lan"
   - Has old property `global: true` instead of `global: false`
   - Cache file shows stale data

3. Sourcing `~/.zshrc` doesn't fix the issue

### Root Cause Investigation

**Cache File Status:**
- Cache hash: `2d06800538d394c2` (stale)
- Config hash: `3fccd0020b136ee9` (current)
- Cache file location: `~/.local/share/aka/aka.json`

**User's Cache Shows:**
```json
{
  "lappy": {
    "name": "lappy",
    "value": "ltl-7007",        // OLD VALUE - should be "ssh ltl-7007.lan"
    "space": true,
    "global": true,              // OLD PROPERTY - should be false
    "count": 4
  },
  "home": {
    "name": "home",
    "value": "ssh desk.lan",
    "count": 1
  }
  // "desk" is MISSING entirely!
}
```

**Current Config:**
```yaml
home|desk: ssh desk.lan
work|lappy: ssh ltl-7007.lan
xps: ssh saidler@xps.lan
```

## The Bugs Identified

### Bug #1: New Aliases From Pipe Separator Inherit Count

**Location:** The interaction between `src/cfg/spec.rs` (parser) and `src/lib.rs` (cache merge)

**What Happens:**
When you modify `home: ssh desk.lan` to `home|desk: ssh desk.lan`:
1. Parser creates two separate `Alias` structs (by cloning)
2. Cache merge finds "home" in old cache → preserves count=1 ✅
3. Cache merge doesn't find "desk" in old cache → should set count=0 ✅
4. **BUT**: The test shows "desk" ends up with count=1 ❌

**Parser Code (`src/cfg/spec.rs:90-102`):**
```rust
while let Some((name, AliasStringOrStruct(mut alias))) = map.next_entry::<String, AliasStringOrStruct>()? {
    let names = if name.starts_with('|') || name.ends_with('|') {
        vec![&name[..]]
    } else {
        name.split('|').collect::<Vec<&str>>()
    };

    for name in names {
        let name = name.to_string();
        alias.name = name.clone();
        aliases.insert(name.clone(), alias.clone());  // Each alias is cloned
    }
}
```

**Cache Merge Code (`src/lib.rs:1094-1101`):**
```rust
for (name, mut alias) in new_spec.aliases {
    // Preserve count if alias existed before
    if let Some(old_alias) = old_cache.aliases.get(&name) {
        alias.count = old_alias.count;  // Only updates count
    }
    new_cache.aliases.insert(name, alias);
}
```

The issue is subtle: when iterating over `new_spec.aliases`, both "home" and "desk" are present as independent entries. The merge SHOULD work correctly - "home" gets count=1, "desk" gets count=0.

However, the test `test_adding_alias_to_existing_entry` fails, showing "desk" has count=1. This suggests there's something wrong with how the aliases are being created or merged that causes count sharing.

### Bug #2: Cache Not Regenerated When Config Changes

**Issue:** The daemon holds the cache in memory and the cache file on disk becomes stale when config changes.

**Why Sourcing .zshrc Doesn't Help:**
- `.zshrc` contains shell functions that call the `aka` binary
- The `aka-daemon` process runs independently
- Sourcing `.zshrc` doesn't reload the daemon
- The daemon continues using old cached values

**Daemon Has File Watcher:**
The daemon includes a file watcher (`src/bin/aka-daemon.rs:74-93`) that should detect config changes and auto-reload. But it seems this isn't working properly or the user hasn't waited for the auto-reload.

## Tests Added

### 1. `tests/alias_modification_caching_bug_test.rs`

**Six tests that expose the caching bugs:**

#### `test_adding_alias_to_existing_entry` ❌ FAILS
- Creates "home" alias with count=1
- Modifies to "home|desk"
- Asserts "desk" should have count=0
- **CURRENTLY FAILS:** "desk" has count=1

#### `test_reusing_alias_name_with_different_value` ✅ PASSES
- Creates "lappy" with old value and count=4
- Redefines as part of "work|lappy" with new value
- Asserts new value is loaded correctly
- **PASSES:** Value is correctly updated

#### `test_alias_value_change_invalidates_count` ✅ PASSES
- Changes alias value
- Asserts new value is loaded
- **PASSES:** Value updates work

#### `test_daemon_reload_with_alias_changes` ✅ PASSES
- Simulates full daemon reload scenario
- Tests all four aliases (home, desk, work, lappy)
- **PASSES:** All aliases resolve correctly

#### `test_cache_merge_updates_changed_values` ✅ PASSES
- Directly tests `merge_cache_with_config_path()`
- Verifies values and properties are updated
- **PASSES:** Merge logic is correct

#### `test_cache_merge_includes_all_pipe_separated_aliases` ✅ PASSES
- Directly tests merge with pipe-separated aliases
- Verifies both aliases exist with correct counts
- **PASSES:** Merge creates both aliases correctly

**Key Insight:** The direct cache merge tests PASS, but the end-to-end test through `AKA::new()` and `sync_cache_with_config_path()` FAILS. This suggests the bug is in how these functions interact, not in the merge logic itself.

### 2. `tests/check_user_cache_test.rs`

**One test that checks the user's actual cache file:**

#### `test_user_cache_is_synchronized` (requires `--ignored` flag)
- Loads user's actual cache file
- Compares with freshly calculated correct cache
- Checks specific aliases (lappy, desk)
- **Initially FAILED:** Cache hash didn't match config hash
- **After running sync:** Cache was regenerated and now matches

## The Mystery - SOLVED!

The test `test_adding_alias_to_existing_entry` was FAILING, but not for the reason we initially thought.

### Root Cause Discovered

**What Actually Happens:**

1. **STEP 5:** `AKA::new()` loads config "home|desk" and syncs cache
   - Cache correctly has: { "home": 1, "desk": 0 } ✅

2. **STEP 5b:** Test uses the "desk" alias: `aka_reloaded.replace_with_mode("desk")`
   - `replace_with_mode()` increments `desk.count` from 0 to 1 in memory
   - **CRITICAL:** `replace_with_mode()` then SAVES the cache immediately (line 764-774 in src/lib.rs)
   - Cache on disk now has: { "home": 1, "desk": 1 }

3. **STEP 6:** Test explicitly calls `sync_cache_with_config_path()` again
   - Loads cache from disk: { "home": 1, "desk": 1 }
   - Parses config fresh: { "home": 0, "desk": 0 }
   - Merges: preserves counts from cache
   - Result: { "home": 1, "desk": 1 } ← This is CORRECT!

4. **Test expects:** "desk" count should be 0
   - **This expectation is WRONG!**
   - We USED the "desk" alias once, so count SHOULD be 1

### The Code That Causes This

**In `src/lib.rs:808-811` - Increment count when alias is used:**
```rust
if replaced {
    // Increment usage count when alias is actually used
    alias.count += 1;
    debug!("📊 Alias '{}' used, count now: {}", alias.name, alias.count);
}
```

**In `src/lib.rs:764-774` - Auto-save cache after ANY alias use:**
```rust
// Save updated usage counts to cache if any aliases were used
if replaced {
    debug!("🔍 SAVING CACHE: Aliases were used, saving cache");
    let cache = AliasCache {
        hash: self.config_hash.clone(),
        aliases: self.spec.aliases.clone(),  // ← Saves ALL aliases with current counts
    };
    if let Err(e) = save_alias_cache(&cache, &self.home_dir) {
        warn!("⚠️ Failed to save alias cache: {}", e);
    }
}
```

### Why Other Tests Pass

- **`test_cache_merge_includes_all_pipe_separated_aliases`**: Directly tests merge without using aliases ✅
- **`test_trace_sequential_syncs`**: Calls sync twice but doesn't USE any aliases between calls ✅

### Conclusion

**This is NOT a caching bug!** The system is working exactly as designed:
1. New aliases start with count=0
2. Using an alias increments its count
3. Cache is saved immediately after use
4. Subsequent syncs preserve the updated counts

The test was exposing correct behavior, not a bug!

## The REAL User Bugs

Now that we understand the test failure, let's identify the ACTUAL user-reported bugs:

### Bug #1: "desk" Alias Missing from Cache

**User's Issue:** Added "desk" to `home|desk: ssh desk.lan` but "desk" doesn't work

**Root Cause:** The user's cache file doesn't contain "desk" at all:
```json
{
  "home": { "count": 1, ... },
  // "desk" is completely missing!
}
```

**Why:** The daemon loaded the old config before "desk" was added, and hasn't reloaded since. The file watcher should have detected the change and auto-reloaded, but either:
1. The file watcher didn't trigger
2. The daemon was restarted and loaded stale cache
3. The config change happened while daemon wasn't running

### Bug #2: "lappy" Has Stale Value

**User's Issue:** Redefined "lappy" from standalone to `work|lappy: ssh ltl-7007.lan` but it still returns old value

**Root Cause:** The cache has the OLD definition:
```json
{
  "lappy": {
    "value": "ltl-7007",     // OLD - should be "ssh ltl-7007.lan"
    "global": true,          // OLD - should be false
    "count": 4
  }
}
```

**Why:** Same as Bug #1 - daemon hasn't reloaded since config changed.

### Bug #3: Sourcing .zshrc Doesn't Help

**User's Expectation:** After modifying config, sourcing `~/.zshrc` should pick up changes

**Reality:** Sourcing .zshrc only reloads shell functions, not the daemon. The daemon is a separate process that needs explicit reload.

**Why Users Are Confused:**
- `.zshrc` typically contains shell aliases and functions
- Users expect config changes to take effect after sourcing
- The daemon continues using old cached values

### The Actual Problem

The cache sync logic IS working correctly. The problem is:

1. **Daemon doesn't auto-reload reliably**
   - File watcher may not trigger
   - Daemon may crash/restart with stale cache
   - No clear indication to user that reload is needed

2. **Users don't know how to reload**
   - `aka daemon --reload` is not intuitive
   - No warning when cache is stale
   - Sourcing .zshrc gives false sense that it should work

3. **Cache staleness is invisible**
   - User changes config
   - Daemon keeps serving old values
   - No error message or warning

### What Our Tests Revealed

The test `test_adding_alias_to_existing_entry` initially appeared to expose a bug, but actually revealed:
1. The merge logic works correctly ✅
2. New aliases start with count=0 ✅
3. Using aliases increments count ✅
4. Cache saves immediately after use ✅

The test was written with wrong expectations (expecting count=0 after using the alias once).

## Proposed Fix

Based on our analysis, the caching and merge logic are working correctly. The REAL issues are:
1. Daemon cache staleness (file watcher not triggering reload)
2. Poor user visibility into cache state
3. Confusing UX (sourcing .zshrc doesn't help)

### Phase 1: Fix Test Expectations

**Current:** `test_adding_alias_to_existing_entry` expects desk count=0 after using it once

**Fix:** Update test to match correct behavior:
```rust
// After using desk once, count should be 1 (not 0)
assert_eq!(cache_reloaded.aliases.get("desk").unwrap().count, 1,
           "desk was used once, count should be 1");
```

OR remove the usage from the test if we want to test initial state:
```rust
// Don't use the desk alias - just check it was created correctly
// let result_desk = aka_reloaded.replace_with_mode("desk", ...)?;  ← Remove this line
assert_eq!(cache_reloaded.aliases.get("desk").unwrap().count, 0,
           "desk was just created, count should be 0");
```

### Phase 2: Improve Daemon Cache Validation

**Problem:** Daemon serves stale cache values without warning

**Solution:** Add proactive cache freshness check before serving queries

```rust
// In daemon's handle_client(), before processing request:
fn ensure_cache_fresh(&self) -> Result<()> {
    let current_hash = hash_config_file(&self.config_path)?;
    let cached_hash = {
        let hash_guard = self.config_hash.read()?;
        hash_guard.clone()
    };

    if current_hash != cached_hash {
        warn!("Config hash mismatch, auto-reloading: {} != {}", current_hash, cached_hash);
        self.reload_config()?;
    }

    Ok(())
}

// Call this before processing Query, List, Freq, etc:
self.ensure_cache_fresh()?;
```

**Benefits:**
- Daemon always serves fresh data
- No reliance on file watcher
- Automatic recovery from stale cache

**Trade-off:**
- Adds ~1ms overhead per query (hash calculation)
- Acceptable for correctness

### Phase 3: Improve File Watcher Reliability

**Current Issues:**
- File watcher may not trigger on some systems
- Users edit config in ways that don't trigger events
- Daemon may miss events if busy

**Improvements:**

1. **Add polling fallback:**
   ```rust
   // Check config hash every 5 seconds as fallback
   let last_check = Arc::new(RwLock::new(Instant::now()));

   // In main loop:
   if last_check.read()?.elapsed() > Duration::from_secs(5) {
       if let Err(e) = self.ensure_cache_fresh() {
           error!("Periodic cache check failed: {}", e);
       }
       *last_check.write()? = Instant::now();
   }
   ```

2. **Log file watcher events clearly:**
   ```rust
   debug!("📁 File watcher detected change in config");
   debug!("🔄 Auto-reloading config...");
   ```

### Phase 4: Better User Communication

**Problem:** Users don't know cache is stale or how to fix it

**Solutions:**

1. **Add --check-cache command:**
   ```bash
   aka daemon --check-cache
   # Output:
   # ✅ Cache is synchronized with config
   # OR
   # ⚠️  Cache is out of sync (config modified: 2025-10-08 14:30)
   #    Run: aka daemon --reload
   ```

2. **Warn in CLI when daemon cache is stale:**
   ```rust
   // In aka CLI, before querying daemon:
   if !daemon_cache_is_fresh() {
       eprintln!("⚠️  Daemon cache may be stale. Run: aka daemon --reload");
   }
   ```

3. **Improve daemon health check output:**
   ```rust
   // Current: "healthy:123:synced"
   // Enhanced: Include age of cache
   format!("healthy:{}:synced:{}s", alias_count, cache_age_seconds)
   ```

4. **Add hint in shell integration:**
   ```bash
   # In aka-loader.zsh, after config edit:
   echo "💡 Config modified. Daemon reload recommended: aka daemon --reload"
   ```

### Phase 5: Fix User's Immediate Problem

**Quick fix for the user:**

```bash
# Force cache regeneration
aka daemon --restart

# OR just reload
aka daemon --reload
```

This will:
1. Re-parse the config with `home|desk` and `work|lappy`
2. Merge with existing cache (preserving counts)
3. Create the missing "desk" alias
4. Update "lappy" with new value and properties

## Testing Strategy

### Unit Tests (Already Added)
- ✅ `test_cache_merge_updates_changed_values` - Tests merge logic directly
- ✅ `test_cache_merge_includes_all_pipe_separated_aliases` - Tests pipe separation
- ❌ `test_adding_alias_to_existing_entry` - Exposes the count inheritance bug

### Integration Tests (To Add)
1. Test daemon reload with config changes
2. Test file watcher triggers reload
3. Test cache invalidation on hash mismatch

### Manual Testing
1. Reproduce user's exact scenario:
   ```bash
   # Start with home: ssh desk.lan
   echo "aliases:\n  home: ssh desk.lan" > ~/.config/aka/aka.yml
   aka daemon --restart
   aka query "home"  # Should work

   # Change to home|desk: ssh desk.lan
   echo "aliases:\n  home|desk: ssh desk.lan" > ~/.config/aka/aka.yml
   aka daemon --reload
   aka query "desk"  # Should work with count=0
   ```

2. Check cache file after each step:
   ```bash
   cat ~/.local/share/aka/aka.json | jq '.aliases | {home, desk}'
   ```

## Implementation Order

1. **Phase 1:** Add diagnostic logging (30 minutes)
   - Understand the exact failure mode
   - Confirm our hypothesis

2. **Phase 2:** Fix count inheritance (1 hour)
   - Implement Option C (explicit count=0 at parse time)
   - Run all tests to verify fix
   - Update test expectations

3. **Phase 3:** Improve daemon sync (2 hours)
   - Add cache validation
   - Improve auto-reload
   - Test file watcher

4. **Phase 4:** User warnings (1 hour)
   - Add helpful messages
   - Update documentation
   - Test UX

## Success Criteria

- ✅ All tests pass
- ✅ `test_adding_alias_to_existing_entry` passes (count=0 for new aliases)
- ✅ User's cache file regenerates correctly
- ✅ Daemon detects config changes and reloads
- ✅ New aliases from pipe-separated names start with count=0
- ✅ Existing aliases preserve their counts
- ✅ Alias values update correctly when config changes

## Open Questions

1. **When alias value changes, should count be reset or preserved?**
   - Reset to 0: More accurate (different command = different alias)
   - Preserve: Historical data across refactoring
   - **Recommendation:** Preserve, but add option for user to reset

2. **Should pipe-separated aliases share counts?**
   - Share: All names point to same command, combined usage
   - Separate: Each name has independent usage tracking
   - **Current behavior:** Separate (each name has own count)
   - **Recommendation:** Keep separate, it's more intuitive

3. **How aggressive should auto-reload be?**
   - Reload on every query (safest but slowest)
   - Reload only when file watcher triggers (faster but may miss changes)
   - Reload on health check (good middle ground)
   - **Recommendation:** File watcher + health check validation

## References

- Issue reported by user on 2025-10-08
- Cache location: `~/.local/share/aka/aka.json`
- Config location: `~/.config/aka/aka.yml`
- Test file: `tests/alias_modification_caching_bug_test.rs`
- Parser code: `src/cfg/spec.rs:90-102`
- Merge code: `src/lib.rs:1094-1101`
- Daemon code: `src/bin/aka-daemon.rs`

