/// Detailed trace of what the parser creates for pipe-separated aliases

use aka_lib::{ConfigLoader, AliasCache, merge_cache_with_config_path, hash_config_file};
use std::fs;
use std::collections::HashMap;
use tempfile::TempDir;
use eyre::Result;

#[test]
fn test_trace_parser_output() -> Result<()> {
    let tmp_dir = TempDir::new()?;
    let config_path = tmp_dir.path().join("config.yml");

    // Create config with pipe-separated aliases
    let config = r#"
aliases:
  home|desk: ssh desk.lan
"#;
    fs::write(&config_path, config)?;

    // Load the config directly
    let loader = ConfigLoader::new();
    let spec = loader.load(&config_path)?;

    eprintln!("\n=== PARSER OUTPUT ===");
    eprintln!("Number of aliases: {}", spec.aliases.len());

    for (name, alias) in &spec.aliases {
        eprintln!("\nAlias name: '{}'", name);
        eprintln!("  alias.name field: '{}'", alias.name);
        eprintln!("  alias.value: '{}'", alias.value);
        eprintln!("  alias.count: {}", alias.count);
        eprintln!("  address: {:p}", alias);
    }

    // Check if they're the same object or different
    let home_alias = spec.aliases.get("home").unwrap();
    let desk_alias = spec.aliases.get("desk").unwrap();

    eprintln!("\n=== ALIAS COMPARISON ===");
    eprintln!("home address: {:p}", home_alias);
    eprintln!("desk address: {:p}", desk_alias);
    eprintln!("Are they the same object? {}", std::ptr::eq(home_alias, desk_alias));

    Ok(())
}

#[test]
fn test_trace_merge_logic() -> Result<()> {
    let tmp_dir = TempDir::new()?;
    let home_dir = tmp_dir.path().to_path_buf();
    let config_path = home_dir.join("config.yml");

    // Create config with pipe-separated aliases
    let config = r#"
aliases:
  home|desk: ssh desk.lan
"#;
    fs::write(&config_path, config)?;
    let new_hash = hash_config_file(&config_path)?;

    // Create old cache with just "home"
    let mut old_aliases = HashMap::new();
    old_aliases.insert("home".to_string(), aka_lib::AliasType {
        name: "home".to_string(),
        value: "ssh desk.lan".to_string(),
        space: true,
        global: false,
        count: 5,
    });
    let old_cache = AliasCache {
        hash: "old_hash".to_string(),
        aliases: old_aliases,
    };

    eprintln!("\n=== OLD CACHE ===");
    for (name, alias) in &old_cache.aliases {
        eprintln!("'{}': count={}", name, alias.count);
    }

    // Merge
    let new_cache = merge_cache_with_config_path(old_cache, new_hash, &config_path)?;

    eprintln!("\n=== NEW CACHE (after merge) ===");
    for (name, alias) in &new_cache.aliases {
        eprintln!("'{}': count={}, value='{}', address={:p}",
                  name, alias.count, alias.value, alias);
    }

    // Test expectations
    assert_eq!(new_cache.aliases.get("home").unwrap().count, 5, "home should preserve count");
    assert_eq!(new_cache.aliases.get("desk").unwrap().count, 0, "desk should have count=0");

    Ok(())
}

#[test]
fn test_trace_sequential_syncs() -> Result<()> {
    let tmp_dir = TempDir::new()?;
    let home_dir = tmp_dir.path().to_path_buf();
    let config_path = home_dir.join("config.yml");

    // Create cache directory
    fs::create_dir_all(home_dir.join(".local/share/aka"))?;

    // Step 1: Config with just "home"
    let config1 = r#"
aliases:
  home: ssh desk.lan
"#;
    fs::write(&config_path, config1)?;
    let hash1 = hash_config_file(&config_path)?;

    // Create initial cache manually
    let mut initial_aliases = HashMap::new();
    initial_aliases.insert("home".to_string(), aka_lib::AliasType {
        name: "home".to_string(),
        value: "ssh desk.lan".to_string(),
        space: true,
        global: false,
        count: 1,
    });
    let initial_cache = AliasCache {
        hash: hash1.clone(),
        aliases: initial_aliases,
    };
    aka_lib::save_alias_cache(&initial_cache, &home_dir)?;

    eprintln!("\n=== INITIAL STATE ===");
    eprintln!("Config: home: ssh desk.lan");
    eprintln!("Cache: home count=1");

    // Step 2: Change config to "home|desk"
    let config2 = r#"
aliases:
  home|desk: ssh desk.lan
"#;
    fs::write(&config_path, config2)?;

    eprintln!("\n=== CHANGED CONFIG ===");
    eprintln!("Config: home|desk: ssh desk.lan");

    // Step 3: First sync
    let cache_after_sync1 = aka_lib::sync_cache_with_config_path(&home_dir, &config_path)?;

    eprintln!("\n=== AFTER FIRST SYNC ===");
    for (name, alias) in &cache_after_sync1.aliases {
        eprintln!("'{}': count={}", name, alias.count);
    }

    // Step 4: Second sync (without changing config)
    let cache_after_sync2 = aka_lib::sync_cache_with_config_path(&home_dir, &config_path)?;

    eprintln!("\n=== AFTER SECOND SYNC ===");
    for (name, alias) in &cache_after_sync2.aliases {
        eprintln!("'{}': count={}", name, alias.count);
    }

    // The counts should be the same after both syncs
    assert_eq!(
        cache_after_sync1.aliases.get("home").unwrap().count,
        cache_after_sync2.aliases.get("home").unwrap().count,
        "home count should not change between syncs"
    );
    assert_eq!(
        cache_after_sync1.aliases.get("desk").unwrap().count,
        cache_after_sync2.aliases.get("desk").unwrap().count,
        "desk count should not change between syncs"
    );

    eprintln!("\n=== FINAL ASSERTIONS ===");
    eprintln!("home count: {} (expected: 1)", cache_after_sync2.aliases.get("home").unwrap().count);
    eprintln!("desk count: {} (expected: 0)", cache_after_sync2.aliases.get("desk").unwrap().count);

    assert_eq!(cache_after_sync2.aliases.get("home").unwrap().count, 1);
    assert_eq!(cache_after_sync2.aliases.get("desk").unwrap().count, 0);

    Ok(())
}

