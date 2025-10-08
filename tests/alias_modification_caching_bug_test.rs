/// Test suite to reproduce the alias caching bug where:
/// 1. Adding a new alias name to an existing entry (home|desk) doesn't work
/// 2. Reusing an alias name with a different value keeps the old cached value
///
/// Bug description:
/// - User had "lappy" defined as one thing, deleted it, and redefined it as part of "work|lappy"
/// - The cached version of "lappy" with the OLD value persists
/// - User added "desk" as an additional alias to "home", but it never works
///
/// The root cause is in src/lib.rs merge_cache_with_config() which only preserves
/// the count but doesn't validate that the value/properties match.

use aka_lib::{AKA, sync_cache_with_config_path, AliasCache, merge_cache_with_config_path, hash_config_file};
use std::fs;
use std::collections::HashMap;
use tempfile::TempDir;
use eyre::Result;

#[test]
fn test_adding_alias_to_existing_entry() -> Result<()> {
    let tmp_dir = TempDir::new()?;
    let home_dir = tmp_dir.path().to_path_buf();
    let config_path = home_dir.join(".config/aka/aka.yml");

    // Create config directory
    fs::create_dir_all(config_path.parent().unwrap())?;

    // Step 1: Create initial config with "home: ssh desk.lan"
    let initial_config = r#"
aliases:
  home: ssh desk.lan
"#;
    fs::write(&config_path, initial_config)?;

    eprintln!("\n=== STEP 1: Initial config created ===");
    eprintln!("Config: home: ssh desk.lan");

    // Load and use the alias to build up a count
    let mut aka = AKA::new(true, home_dir.clone(), config_path.clone())?;
    let result = aka.replace_with_mode("home", aka_lib::ProcessingMode::Direct)?;
    assert_eq!(result.trim(), "ssh desk.lan");

    eprintln!("\n=== STEP 2: Used 'home' alias once ===");
    eprintln!("AKA.spec.aliases keys: {:?}", aka.spec.aliases.keys().collect::<Vec<_>>());
    eprintln!("home count in spec: {}", aka.spec.aliases.get("home").unwrap().count);

    // Sync cache to persist the count
    let cache = sync_cache_with_config_path(&home_dir, &config_path)?;
    eprintln!("\n=== STEP 3: Synced cache ===");
    eprintln!("Cache keys: {:?}", cache.aliases.keys().collect::<Vec<_>>());
    eprintln!("home count in cache: {}", cache.aliases.get("home").unwrap().count);
    assert_eq!(cache.aliases.get("home").unwrap().count, 1);
    assert!(cache.aliases.get("desk").is_none()); // "desk" doesn't exist yet

    // Step 2: Modify config to add "desk" as an additional alias: "home|desk: ssh desk.lan"
    let modified_config = r#"
aliases:
  home|desk: ssh desk.lan
"#;
    fs::write(&config_path, modified_config)?;

    eprintln!("\n=== STEP 4: Modified config ===");
    eprintln!("Config: home|desk: ssh desk.lan");

    // Step 3: Reload and verify "desk" exists
    let aka_reloaded = AKA::new(true, home_dir.clone(), config_path.clone())?;

    eprintln!("\n=== STEP 5: Reloaded AKA ===");
    eprintln!("AKA.spec.aliases keys: {:?}", aka_reloaded.spec.aliases.keys().collect::<Vec<_>>());
    eprintln!("home count in spec: {}", aka_reloaded.spec.aliases.get("home").unwrap().count);
    eprintln!("desk count in spec: {}", aka_reloaded.spec.aliases.get("desk").unwrap().count);

    // Verify the alias exists and works (but don't use it to avoid incrementing count)
    assert!(aka_reloaded.spec.aliases.contains_key("desk"), "desk should exist in spec");
    assert_eq!(aka_reloaded.spec.aliases.get("desk").unwrap().value, "ssh desk.lan");

    // Step 4: Sync cache again (without using the alias)
    let cache_reloaded = sync_cache_with_config_path(&home_dir, &config_path)?;

    eprintln!("\n=== STEP 6: Synced cache after reload ===");
    eprintln!("Cache keys: {:?}", cache_reloaded.aliases.keys().collect::<Vec<_>>());
    eprintln!("home count in cache: {}", cache_reloaded.aliases.get("home").unwrap().count);
    eprintln!("desk count in cache: {}", cache_reloaded.aliases.get("desk").unwrap().count);

    // Both "home" and "desk" should exist now
    assert!(cache_reloaded.aliases.get("home").is_some(), "home should exist in cache");
    assert!(cache_reloaded.aliases.get("desk").is_some(), "desk should exist in cache");

    // Both should have the same value
    assert_eq!(cache_reloaded.aliases.get("home").unwrap().value, "ssh desk.lan");
    assert_eq!(cache_reloaded.aliases.get("desk").unwrap().value, "ssh desk.lan");

    // "home" should preserve its count
    assert_eq!(cache_reloaded.aliases.get("home").unwrap().count, 1, "home should preserve count");

    // "desk" is new, so count MUST be 0
    assert_eq!(cache_reloaded.aliases.get("desk").unwrap().count, 0,
               "desk is new and should start with count=0");

    Ok(())
}

#[test]
fn test_reusing_alias_name_with_different_value() -> Result<()> {
    let tmp_dir = TempDir::new()?;
    let home_dir = tmp_dir.path().to_path_buf();
    let config_path = home_dir.join(".config/aka/aka.yml");

    // Create config directory
    fs::create_dir_all(config_path.parent().unwrap())?;

    // Step 1: Create initial config with "lappy" as a standalone alias
    let initial_config = r#"
aliases:
  lappy:
    value: "ltl-7007"
    global: true
"#;
    fs::write(&config_path, initial_config)?;

    // Load and use the alias multiple times to build up a count
    let mut aka = AKA::new(true, home_dir.clone(), config_path.clone())?;
    for _ in 0..4 {
        let _result = aka.replace_with_mode("lappy", aka_lib::ProcessingMode::Direct)?;
    }

    // Sync cache to persist the count
    let cache = sync_cache_with_config_path(&home_dir, &config_path)?;
    let lappy_cached = cache.aliases.get("lappy").unwrap();
    assert_eq!(lappy_cached.value, "ltl-7007");
    assert_eq!(lappy_cached.count, 4);
    assert_eq!(lappy_cached.global, true);

    // Step 2: Delete "lappy" and redefine it as part of "work|lappy"
    let modified_config = r#"
aliases:
  work|lappy: ssh ltl-7007.lan
"#;
    fs::write(&config_path, modified_config)?;

    // Step 3: Reload and try to use "lappy"
    let mut aka_reloaded = AKA::new(true, home_dir.clone(), config_path.clone())?;

    // BUG: "lappy" should have the NEW value but might have cached old value
    let result_lappy = aka_reloaded.replace_with_mode("lappy", aka_lib::ProcessingMode::Direct)?;
    assert_eq!(result_lappy.trim(), "ssh ltl-7007.lan",
               "lappy should resolve to NEW value, not old cached value");

    // Step 4: Sync cache again
    let cache_reloaded = sync_cache_with_config_path(&home_dir, &config_path)?;

    let lappy_reloaded = cache_reloaded.aliases.get("lappy").unwrap();

    // BUG: The value should be updated
    assert_eq!(lappy_reloaded.value, "ssh ltl-7007.lan",
               "lappy value should be updated to ssh ltl-7007.lan");

    // The properties should be updated too
    assert_eq!(lappy_reloaded.global, false,
               "lappy global property should be updated to false");

    // The count should be RESET because it's a different alias now
    // OR at minimum, we should detect the value changed and invalidate the count
    // For now, let's just verify the value is correct

    Ok(())
}

#[test]
fn test_alias_value_change_invalidates_count() -> Result<()> {
    let tmp_dir = TempDir::new()?;
    let home_dir = tmp_dir.path().to_path_buf();
    let config_path = home_dir.join(".config/aka/aka.yml");

    // Create config directory
    fs::create_dir_all(config_path.parent().unwrap())?;

    // Step 1: Create initial config
    let initial_config = r#"
aliases:
  myalias: echo hello
"#;
    fs::write(&config_path, initial_config)?;

    // Use it multiple times
    let mut aka = AKA::new(true, home_dir.clone(), config_path.clone())?;
    for _ in 0..10 {
        let _result = aka.replace_with_mode("myalias", aka_lib::ProcessingMode::Direct)?;
    }

    // Sync cache
    let cache = sync_cache_with_config_path(&home_dir, &config_path)?;
    assert_eq!(cache.aliases.get("myalias").unwrap().count, 10);
    assert_eq!(cache.aliases.get("myalias").unwrap().value, "echo hello");

    // Step 2: Change the value completely
    let modified_config = r#"
aliases:
  myalias: echo goodbye
"#;
    fs::write(&config_path, modified_config)?;

    // Step 3: Reload
    let mut aka_reloaded = AKA::new(true, home_dir.clone(), config_path.clone())?;
    let result = aka_reloaded.replace_with_mode("myalias", aka_lib::ProcessingMode::Direct)?;
    assert_eq!(result.trim(), "echo goodbye", "myalias should have new value");

    // Step 4: Sync cache again
    let cache_reloaded = sync_cache_with_config_path(&home_dir, &config_path)?;
    let myalias_reloaded = cache_reloaded.aliases.get("myalias").unwrap();

    assert_eq!(myalias_reloaded.value, "echo goodbye", "value should be updated");

    // DESIGN DECISION: Should count be reset when value changes?
    // Option 1: Reset to 0 (alias is "new")
    // Option 2: Preserve count (usage tracking is independent of value)
    //
    // For now, this test just ensures the value is correct.
    // The bug is that the value WASN'T being updated in the cache.

    Ok(())
}

#[test]
fn test_daemon_reload_with_alias_changes() -> Result<()> {
    // This test simulates the user's scenario with the daemon running
    let tmp_dir = TempDir::new()?;
    let home_dir = tmp_dir.path().to_path_buf();
    let config_path = home_dir.join(".config/aka/aka.yml");

    // Create config directory
    fs::create_dir_all(config_path.parent().unwrap())?;

    // Step 1: Start with initial config
    let initial_config = r#"
aliases:
  home: ssh desk.lan
  lappy:
    value: "ltl-7007"
    global: true
"#;
    fs::write(&config_path, initial_config)?;

    // Create initial AKA instance (simulating daemon startup)
    let mut aka = AKA::new(true, home_dir.clone(), config_path.clone())?;

    // Use aliases to build counts
    aka.replace_with_mode("home", aka_lib::ProcessingMode::Direct)?;
    aka.replace_with_mode("lappy", aka_lib::ProcessingMode::Direct)?;
    aka.replace_with_mode("lappy", aka_lib::ProcessingMode::Direct)?;
    aka.replace_with_mode("lappy", aka_lib::ProcessingMode::Direct)?;

    // Save cache
    let cache = sync_cache_with_config_path(&home_dir, &config_path)?;
    assert_eq!(cache.aliases.get("home").unwrap().count, 1);
    assert_eq!(cache.aliases.get("lappy").unwrap().count, 3);

    // Step 2: User modifies config
    let modified_config = r#"
aliases:
  home|desk: ssh desk.lan
  work|lappy: ssh ltl-7007.lan
"#;
    fs::write(&config_path, modified_config)?;

    // Step 3: Reload config (simulating daemon reload or restart)
    let mut aka_reloaded = AKA::new(true, home_dir.clone(), config_path.clone())?;

    // Test all four aliases work with correct values
    let result_home = aka_reloaded.replace_with_mode("home", aka_lib::ProcessingMode::Direct)?;
    assert_eq!(result_home.trim(), "ssh desk.lan", "home should work");

    let result_desk = aka_reloaded.replace_with_mode("desk", aka_lib::ProcessingMode::Direct)?;
    assert_eq!(result_desk.trim(), "ssh desk.lan", "desk should work (new alias)");

    let result_work = aka_reloaded.replace_with_mode("work", aka_lib::ProcessingMode::Direct)?;
    assert_eq!(result_work.trim(), "ssh ltl-7007.lan", "work should work");

    let result_lappy = aka_reloaded.replace_with_mode("lappy", aka_lib::ProcessingMode::Direct)?;
    assert_eq!(result_lappy.trim(), "ssh ltl-7007.lan",
               "lappy should have NEW value, not old cached value");

    // Verify cache is correct
    let cache_reloaded = sync_cache_with_config_path(&home_dir, &config_path)?;

    // "home" and "desk" should both exist with same value
    assert_eq!(cache_reloaded.aliases.get("home").unwrap().value, "ssh desk.lan");
    assert_eq!(cache_reloaded.aliases.get("desk").unwrap().value, "ssh desk.lan");

    // "work" and "lappy" should both exist with same value
    assert_eq!(cache_reloaded.aliases.get("work").unwrap().value, "ssh ltl-7007.lan");
    assert_eq!(cache_reloaded.aliases.get("lappy").unwrap().value, "ssh ltl-7007.lan");

    Ok(())
}

#[test]
fn test_cache_merge_updates_changed_values() -> Result<()> {
    // This test directly tests the merge_cache_with_config_path function
    // to ensure it updates values when they change
    let tmp_dir = TempDir::new()?;
    let home_dir = tmp_dir.path().to_path_buf();
    let config_path = home_dir.join(".config/aka/aka.yml");

    // Create config directory
    fs::create_dir_all(config_path.parent().unwrap())?;

    // Create a config with old value
    let old_config = r#"
aliases:
  lappy:
    value: "ltl-7007"
    global: true
"#;
    fs::write(&config_path, old_config)?;

    // Create a cache with the old value
    let old_hash = hash_config_file(&config_path)?;
    let mut old_aliases = HashMap::new();
    old_aliases.insert("lappy".to_string(), aka_lib::AliasType {
        name: "lappy".to_string(),
        value: "ltl-7007".to_string(),
        space: true,
        global: true,
        count: 4,
    });
    let old_cache = AliasCache {
        hash: old_hash.clone(),
        aliases: old_aliases,
    };

    // Now change the config to have a NEW value
    let new_config = r#"
aliases:
  lappy: ssh ltl-7007.lan
"#;
    fs::write(&config_path, new_config)?;
    let new_hash = hash_config_file(&config_path)?;

    // Merge the cache
    let merged_cache = merge_cache_with_config_path(old_cache, new_hash, &config_path)?;

    // The merged cache MUST have the NEW value
    let lappy = merged_cache.aliases.get("lappy").unwrap();
    assert_eq!(lappy.value, "ssh ltl-7007.lan",
               "Cache merge must update value when it changes in config");

    // The count should be preserved
    assert_eq!(lappy.count, 4, "Count should be preserved");

    // The global property should be updated
    assert_eq!(lappy.global, false,
               "Cache merge must update properties when they change in config");

    Ok(())
}

#[test]
fn test_cache_merge_includes_all_pipe_separated_aliases() -> Result<()> {
    // This test ensures that when you add "desk" to "home|desk",
    // both "home" and "desk" appear in the cache
    let tmp_dir = TempDir::new()?;
    let home_dir = tmp_dir.path().to_path_buf();
    let config_path = home_dir.join(".config/aka/aka.yml");

    // Create config directory
    fs::create_dir_all(config_path.parent().unwrap())?;

    // Create old config with just "home"
    let old_config = r#"
aliases:
  home: ssh desk.lan
"#;
    fs::write(&config_path, old_config)?;
    let old_hash = hash_config_file(&config_path)?;

    let mut old_aliases = HashMap::new();
    old_aliases.insert("home".to_string(), aka_lib::AliasType {
        name: "home".to_string(),
        value: "ssh desk.lan".to_string(),
        space: true,
        global: false,
        count: 5,
    });
    let old_cache = AliasCache {
        hash: old_hash,
        aliases: old_aliases,
    };

    // Now change config to "home|desk"
    let new_config = r#"
aliases:
  home|desk: ssh desk.lan
"#;
    fs::write(&config_path, new_config)?;
    let new_hash = hash_config_file(&config_path)?;

    // Merge the cache
    let merged_cache = merge_cache_with_config_path(old_cache, new_hash, &config_path)?;

    // Both "home" and "desk" MUST be in the cache
    assert!(merged_cache.aliases.contains_key("home"),
            "home must be in cache");
    assert!(merged_cache.aliases.contains_key("desk"),
            "desk must be in cache");

    // Both should have the same value
    assert_eq!(merged_cache.aliases.get("home").unwrap().value, "ssh desk.lan");
    assert_eq!(merged_cache.aliases.get("desk").unwrap().value, "ssh desk.lan");

    // "home" should preserve its count
    assert_eq!(merged_cache.aliases.get("home").unwrap().count, 5,
               "home should preserve its count");

    // "desk" is NEW, should have count=0
    assert_eq!(merged_cache.aliases.get("desk").unwrap().count, 0,
               "desk is new and must start with count=0");

    Ok(())
}

