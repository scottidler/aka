/// Test that checks if the user's actual cache file is stale
/// This test will FAIL if the cache needs to be regenerated

use aka_lib::{sync_cache_with_config_path, load_alias_cache, hash_config_file, get_config_path};
use eyre::Result;

#[test]
#[ignore] // Ignore by default since it touches user's actual files
fn test_user_cache_is_synchronized() -> Result<()> {
    // Get user's actual home directory
    let home_dir = dirs::home_dir().expect("Could not get home directory");
    let config_path = get_config_path(&home_dir)?;

    // Load the current cache
    let current_cache = load_alias_cache(&home_dir)?;

    // Calculate what the cache SHOULD be
    let correct_cache = sync_cache_with_config_path(&home_dir, &config_path)?;

    // Calculate current config hash
    let current_hash = hash_config_file(&config_path)?;

    println!("Current cache hash: {}", current_cache.hash);
    println!("Current config hash: {}", current_hash);
    println!("Current cache aliases: {}", current_cache.aliases.len());
    println!("Correct cache aliases: {}", correct_cache.aliases.len());

    // The hashes should match
    assert_eq!(current_cache.hash, current_hash,
               "Cache hash doesn't match config hash - cache is stale!");

    // Check specific aliases that the user mentioned
    if let (Some(lappy_cached), Some(lappy_correct)) =
        (current_cache.aliases.get("lappy"), correct_cache.aliases.get("lappy")) {
        println!("lappy cached value: {}", lappy_cached.value);
        println!("lappy correct value: {}", lappy_correct.value);
        assert_eq!(lappy_cached.value, lappy_correct.value,
                   "lappy has wrong value in cache");
        assert_eq!(lappy_cached.global, lappy_correct.global,
                   "lappy has wrong global property in cache");
    }

    // Check if "desk" exists
    assert!(correct_cache.aliases.contains_key("desk"),
            "desk should exist in correct cache");
    if !current_cache.aliases.contains_key("desk") {
        panic!("desk is missing from current cache but should exist!");
    }

    Ok(())
}

