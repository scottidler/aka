use eyre::{eyre, Result};
use log::{info, debug};
use std::fs::OpenOptions;
use std::path::PathBuf;
use xxhash_rust::xxh3::xxh3_64;

pub mod cfg;
use cfg::alias::Alias;
use cfg::loader::Loader;
use cfg::spec::Spec;

// Re-export for binaries
pub use cfg::alias::Alias as AliasType;
pub use cfg::loader::Loader as ConfigLoader;
pub use cfg::spec::Spec as ConfigSpec;

pub fn get_config_path() -> Result<PathBuf> {
    let config_path = dirs::config_dir()
        .ok_or_else(|| eyre!("Could not determine config directory"))?
        .join("aka")
        .join("aka.yml");

    if config_path.exists() {
        Ok(config_path)
    } else {
        eprintln!("Error: Config file not found at {:?}", config_path);
        eprintln!("Please create the config file first.");
        Err(eyre!("Config file {:?} not found", config_path))
    }
}

pub fn test_config(file: &PathBuf) -> Result<PathBuf> {
    if file.exists() {
        return Ok(file.clone());
    }
    Err(eyre!("config {:?} not found!", file))
}

pub fn setup_logging() -> Result<()> {
    let log_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre!("Could not determine local data directory"))?
        .join("aka")
        .join("logs");

    std::fs::create_dir_all(&log_dir)?;
    let log_file_path = log_dir.join("aka.log");

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)?;

    env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
        .target(env_logger::Target::Pipe(Box::new(log_file)))
        .init();

    Ok(())
}

pub fn get_hash_cache_path() -> Result<PathBuf> {
    let cache_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre!("Could not determine local data directory"))?
        .join("aka");

    std::fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir.join("config.hash"))
}

pub fn hash_config_file(config_path: &PathBuf) -> Result<String> {
    let content = std::fs::read(config_path)?;
    let hash = xxh3_64(&content);
    Ok(format!("{:016x}", hash))
}

pub fn get_stored_hash() -> Result<Option<String>> {
    let hash_path = get_hash_cache_path()?;
    if hash_path.exists() {
        let stored_hash = std::fs::read_to_string(&hash_path)?;
        Ok(Some(stored_hash.trim().to_string()))
    } else {
        Ok(None)
    }
}

pub fn store_hash(hash: &str) -> Result<()> {
    let hash_path = get_hash_cache_path()?;
    std::fs::write(&hash_path, hash)?;
    Ok(())
}

pub fn execute_health_check(config: &Option<PathBuf>) -> Result<i32> {
    // Step 1: Check if config file exists
    let config_path = match config {
        Some(file) => {
            if !file.exists() {
                debug!("Health check failed: specified config file {:?} not found", file);
                return Ok(1); // Config file not found
            }
            file.clone()
        }
        None => {
            let default_config = get_config_path();
            match default_config {
                Ok(path) => path,
                Err(_) => {
                    debug!("Health check failed: no config file found");
                    return Ok(1); // Config file not found
                }
            }
        }
    };

    // Step 2: Calculate current config hash
    let current_hash = match hash_config_file(&config_path) {
        Ok(hash) => hash,
        Err(e) => {
            debug!("Health check failed: cannot read config file: {}", e);
            return Ok(1); // Cannot read config file
        }
    };

    // Step 3: Compare with stored hash
    let stored_hash = get_stored_hash().unwrap_or(None);

    if let Some(stored) = stored_hash {
        if stored == current_hash {
            // Hash matches, config is valid
            debug!("Health check passed: config hash matches");
            return Ok(0);
        }
    }

    // Step 4: Hash doesn't match or no stored hash, validate config
    debug!("Health check: validating config file");

    // Try to load and parse the config
    let loader = Loader::new();
    match loader.load(&config_path) {
        Ok(spec) => {
            // Config is valid, store the new hash
            if let Err(e) = store_hash(&current_hash) {
                debug!("Warning: could not store config hash: {}", e);
            }

            // Check if we have any aliases
            if spec.aliases.is_empty() {
                debug!("Health check passed: config valid but no aliases defined");
                return Ok(3); // No aliases defined
            }

            debug!("Health check passed: config valid with {} aliases", spec.aliases.len());
            Ok(0) // All good
        }
        Err(e) => {
            debug!("Health check failed: config file invalid: {}", e);
            Ok(2) // Config file invalid
        }
    }
}

// Main AKA struct and implementation
pub struct AKA {
    pub eol: bool,
    pub spec: Spec,
}

impl AKA {
    pub fn new(eol: bool, config: &Option<PathBuf>) -> Result<Self> {
        let config_path = match config {
            Some(file) => test_config(file)?,
            None => get_config_path()?,
        };

        let loader = Loader::new();
        let spec = loader.load(&config_path)?;

        Ok(AKA { eol, spec })
    }

    pub fn use_alias(&self, alias: &Alias, pos: usize) -> bool {
        if alias.is_variadic() && !self.eol {
            false
        } else if pos == 0 {
            true
        } else {
            alias.global
        }
    }

    fn split_respecting_quotes(cmdline: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut start = 0;
        let mut in_quotes = false;
        let chars: Vec<char> = cmdline.chars().collect();
        for index in 0..chars.len() {
            if chars[index] == '"' {
                in_quotes = !in_quotes;
            } else if chars[index] == ' ' && !in_quotes {
                if start != index {
                    args.push(cmdline[start..index].to_string());
                }
                start = index + 1;
            } else if chars[index] == '!' && !in_quotes && index == chars.len() - 1 {
                if start != index {
                    args.push(cmdline[start..index].to_string());
                }
                args.push(String::from("!"));
                start = index + 1;
            }
        }
        if start != chars.len() {
            args.push(cmdline[start..].to_string());
        }
        args
    }

    fn perform_lookup(&self, key: &str, lookup: &str) -> Option<String> {
        self.spec.lookups.get(lookup).and_then(|map| map.get(key).cloned())
    }

    pub fn replace(&self, cmdline: &str) -> Result<String> {
        debug!("Processing command line: {}", cmdline);
        let mut pos: usize = 0;
        let mut space = " ";
        let mut replaced = false;
        let mut sudo = false;
        let mut args = Self::split_respecting_quotes(cmdline);

        if self.eol && !args.is_empty() {
            if let Some(last_arg) = args.last() {
                if last_arg == "!" || last_arg.ends_with("!") {
                    args.pop();
                    sudo = true;
                } else if last_arg.starts_with("!") {
                    let next_arg = last_arg[1..].to_string();
                    args[0] = next_arg;
                    replaced = true;

                    let mut i = 1;
                    while i < args.len() {
                        if args[i].starts_with("-") {
                            args.remove(i);
                        } else if args[i] == "|" || args[i] == ">" || args[i] == "<" {
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    args.pop();
                }
            }
        }

        while pos < args.len() {
            let current_arg = args[pos].clone(); // Clone to avoid borrowing conflicts

            // Perform lookup replacement logic
            if current_arg.starts_with("lookup:") && current_arg.contains("[") && current_arg.ends_with("]") {
                let parts: Vec<&str> = current_arg.splitn(2, '[').collect();
                let lookup = parts[0].trim_start_matches("lookup:");
                let key = parts[1].trim_end_matches("]");
                if let Some(replacement) = self.perform_lookup(key, lookup) {
                    args[pos] = replacement.clone(); // Replace in args
                    replaced = true;
                    continue; // Reevaluate the current position after replacement
                }
            }

            let mut remainders: Vec<String> = args[pos + 1..].to_vec();
            let (value, count) = match self.spec.aliases.get(&current_arg) {
                Some(alias) if self.use_alias(alias, pos) => {
                    if (alias.global && cmdline.contains(&alias.value))
                        || (!alias.global && pos == 0 && cmdline.starts_with(&alias.value))
                    {
                        (current_arg.clone(), 0)
                    } else {
                        space = if alias.space { " " } else { "" };
                        let (v, c) = alias.replace(&mut remainders)?;
                        if v != alias.name {
                            replaced = true;
                        }
                        (v, c)
                    }
                }
                Some(_) | None => (current_arg.clone(), 0),
            };

            let beg = pos + 1;
            let end = beg + count;

            if space.is_empty() {
                args.drain(beg..end);
            } else {
                args.drain(beg..end);
            }
            args.splice(pos..=pos, Self::split_respecting_quotes(&value));
            pos += 1;
        }

        if sudo {
            args[0] = format!("$(which {})", args[0]);
            args.insert(0, "sudo".to_string());
        }

        let result = if replaced || sudo {
            format!("{}{}", args.join(" "), space)
        } else {
            String::new()
        };

        if replaced || sudo {
            info!("Command line transformed: {} -> {}", cmdline, result.trim());
        }

        Ok(result)
    }
}

pub fn print_alias(alias: &Alias) {
    if alias.value.contains('\n') {
        println!("{}: |\n  {}", alias.name, alias.value.replace("\n", "\n  "));
    } else {
        println!("{}: {}", alias.name, alias.value);
    }
}

// Utility function to determine socket path for daemon
pub fn determine_socket_path() -> Result<PathBuf> {
    // Try XDG_RUNTIME_DIR first
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = PathBuf::from(runtime_dir).join("aka").join("daemon.sock");
        return Ok(path);
    }

    // Fallback to ~/.local/share/aka/
    let home_dir = dirs::home_dir()
        .ok_or_else(|| eyre!("Could not determine home directory"))?;

    Ok(home_dir.join(".local/share/aka/daemon.sock"))
} 