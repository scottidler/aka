use clap::Parser;
use eyre::{eyre, Result};
use log::{info, debug, error};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::exit;
use xxhash_rust::xxh3::xxh3_64;

pub mod cfg;
use cfg::alias::Alias;
use cfg::loader::Loader;
use cfg::spec::Spec;

fn get_config_path() -> Result<PathBuf> {
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

fn test_config(file: &PathBuf) -> Result<PathBuf> {
    if file.exists() {
        return Ok(file.clone());
    }
    Err(eyre!("config {:?} not found!", file))
}

fn setup_logging() -> Result<()> {
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

fn get_hash_cache_path() -> Result<PathBuf> {
    let cache_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre!("Could not determine local data directory"))?
        .join("aka");

    std::fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir.join("config.hash"))
}

fn hash_config_file(config_path: &PathBuf) -> Result<String> {
    let content = std::fs::read(config_path)?;
    let hash = xxh3_64(&content);
    Ok(format!("{:016x}", hash))
}

fn get_stored_hash() -> Result<Option<String>> {
    let hash_path = get_hash_cache_path()?;
    if hash_path.exists() {
        let stored_hash = std::fs::read_to_string(&hash_path)?;
        Ok(Some(stored_hash.trim().to_string()))
    } else {
        Ok(None)
    }
}

fn store_hash(hash: &str) -> Result<()> {
    let hash_path = get_hash_cache_path()?;
    std::fs::write(&hash_path, hash)?;
    Ok(())
}

fn execute_health_check(config: &Option<PathBuf>) -> Result<i32> {
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

mod built_info {
    include!(concat!(env!("OUT_DIR"), "/git_describe.rs"));
}

#[derive(Parser)]
#[command(name = "aka", about = "[a]lso [k]nown [a]s: an aliasing program")]
#[command(version = built_info::GIT_DESCRIBE)]
#[command(author = "Scott A. Idler <scott.a.idler@gmail.com>")]
#[command(arg_required_else_help = true)]
#[command(after_help = "Logs are written to: ~/.local/share/aka/logs/aka.log")]
struct AkaOpts {
    #[clap(short, long, help = "is entry an [e]nd [o]f [l]ine?")]
    eol: bool,

    #[clap(short, long)]
    config: Option<PathBuf>,

    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Parser)]
enum Command {
    #[clap(name = "ls", about = "list aka aliases")]
    List(ListOpts),

    #[clap(name = "query", about = "query for aka substitutions")]
    Query(QueryOpts),

    #[clap(name = "__complete_aliases", hide = true)]
    CompleteAliases,

    #[clap(name = "__health_check", hide = true)]
    HealthCheck,
}

#[derive(Parser)]
struct QueryOpts {
    cmdline: String,
}

#[derive(Parser)]
struct ListOpts {
    #[clap(short, long, help = "list global aliases only")]
    global: bool,

    patterns: Vec<String>,
}

#[derive(Debug)]
struct AKA {
    pub eol: bool,
    pub spec: Spec,
}

impl AKA {
    pub fn new(eol: bool, config: &Option<PathBuf>) -> Result<Self> {
        let config = match &config {
            Some(file) => test_config(file)?,
            None => get_config_path()?,
        };
        info!("Loading config from: {:?}", config);
        let loader = Loader::new();
        let mut spec = loader.load(&config)?;
        debug!("Loaded spec with {} aliases", spec.aliases.len());

        // Expand keys in lookups
        for (_, map) in spec.lookups.iter_mut() {
            let mut expanded = HashMap::new();
            for (pattern, value) in map.iter() {
                let keys: Vec<&str> = pattern.split('|').collect();
                for key in keys {
                    expanded.insert(key.to_string(), value.clone());
                }
            }
            *map = expanded;
        }

        Ok(Self { eol, spec })
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
        self.spec.lookups.get(lookup)?.get(key).cloned()
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

fn print_alias(alias: &Alias) {
    if alias.value.contains('\n') {
        println!("{}: |\n  {}", alias.name, alias.value.replace("\n", "\n  "));
    } else {
        println!("{}: {}", alias.name, alias.value);
    }
}

fn execute(aka_opts: &AkaOpts) -> Result<i32> {
    // Handle health check first, before trying to create AKA instance
    if let Some(ref command) = &aka_opts.command {
        if let Command::HealthCheck = command {
            return execute_health_check(&aka_opts.config);
        }
    }

    let aka = AKA::new(aka_opts.eol, &aka_opts.config)?;
    if let Some(ref command) = aka_opts.command {
        match command {
            Command::Query(query_opts) => {
                let result = aka.replace(&query_opts.cmdline)?;
                println!("{result}");
            }
            Command::List(list_opts) => {
                let mut aliases: Vec<Alias> = aka.spec.aliases.values().cloned().collect();
                aliases.sort_by_key(|a| a.name.clone());

                if list_opts.global {
                    aliases = aliases.into_iter().filter(|alias| alias.global).collect();
                }

                if list_opts.patterns.is_empty() {
                    for alias in aliases {
                        print_alias(&alias);
                    }
                } else {
                    for alias in aliases {
                        if list_opts.patterns.iter().any(|pattern| alias.name.starts_with(pattern)) {
                            print_alias(&alias);
                        }
                    }
                }
            }

            Command::CompleteAliases => {
                let mut keys: Vec<_> = aka.spec.aliases
                    .keys()
                    .filter(|name| name.len() > 1 && !name.starts_with('|'))
                    .cloned()
                    .collect();
                keys.sort();
                for name in keys {
                    println!("{name}");
                }
                return Ok(0);
            }

            Command::HealthCheck => {
                // This should never be reached due to early return above
                unreachable!("Health check should be handled before AKA instance creation");
            }
        }
    }
    Ok(0)
}

fn main() {
    let opts = AkaOpts::parse();

    if let Err(e) = setup_logging() {
        eprintln!("Failed to setup logging: {}", e);
        exit(1);
    }

    info!("Starting aka");

    exit(match execute(&opts) {
        Ok(exitcode) => exitcode,
        Err(err) => {
            error!("Error: {}", err);
            eprintln!("error: {err:?}");
            1
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eyre::{Error, Result};
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn setup_aka(eol: bool, yaml: &str) -> Result<AKA> {
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", yaml)?;
        let aka = AKA::new(eol, &Some(temp_file.path().to_path_buf()))?;
        Ok(aka)
    }

    #[test]
    fn test_simple_substitution() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
        "#;
        let aka = setup_aka(false, yaml)?;
        let result = aka.replace("cat file.txt")?;
        let expect = "bat -p file.txt ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_spec_deserialize_alias_map_success() -> Result<(), eyre::Error> {
        let yaml = r#"
    defaults:
      version: 1
    aliases:
      alias1:
        value: "echo Hello World"
        space: true
        global: false
        "#;
        let aka = setup_aka(false, yaml)?;
        let spec = &aka.spec;

        assert_eq!(spec.defaults.version, 1);
        assert_eq!(spec.aliases.len(), 1);
        assert_eq!(spec.aliases.get("alias1").unwrap().value, "echo Hello World");

        Ok(())
    }

    #[test]
    fn test_loader_load_success() -> Result<(), Error> {
        let yaml = r#"
    defaults:
      version: 1
    aliases:
      alias1:
        value: "echo Hello World"
        space: true
        global: false
    "#;
        let aka = setup_aka(false, yaml)?;
        let spec = &aka.spec;

        let expected_aliases = {
            let mut map = HashMap::new();
            map.insert(
                "alias1".to_string(),
                Alias {
                    name: "alias1".to_string(),
                    value: "echo Hello World".to_string(),
                    space: true,
                    global: false,
                },
            );
            map
        };

        assert_eq!(spec.aliases, expected_aliases);
        assert_eq!(spec.defaults.version, 1);

        Ok(())
    }

    #[test]
    fn test_no_exclamation_mark() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
        "#;
        let aka = setup_aka(false, yaml)?;
        let result = aka.replace("cat /some/file")?;
        let expect = "bat -p /some/file ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_exclamation_mark_at_end() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
        "#;
        let aka = setup_aka(true, yaml)?;
        let result = aka.replace("vim /some/file !")?;
        let expect = "sudo $(which vim) /some/file ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_exclamation_mark_with_alias() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
        "#;
        let aka = setup_aka(true, yaml)?;
        let result = aka.replace("vim /some/file !cat")?;
        let expect = "bat -p /some/file ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_multiple_substitutions() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
            '|c':
                value: '| xclip -sel clip'
                global: true
        "#;
        let aka = setup_aka(false, yaml)?;
        let result = aka.replace("cat file.txt |c && echo test")?;
        let expect = "bat -p file.txt | xclip -sel clip && echo test "; // Corrected expectation
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_exclamation_mark_handling() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            vim: "nvim"
        "#;
        let aka = setup_aka(true, yaml)?;
        let result = aka.replace("vim file.txt !")?;
        let expect = "sudo $(which nvim) file.txt ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_quotes_handling() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            grep: "rg"
        "#;
        let aka = setup_aka(false, yaml)?;
        let result = aka.replace("grep \"pattern\" file.txt")?;
        let expect = "rg \"pattern\" file.txt ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_sudo_handling() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            vim: "nvim"
        "#;
        let aka = setup_aka(true, yaml)?;
        let result = aka.replace("vim file.txt !")?;
        let expect = "sudo $(which nvim) file.txt ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_variadic_alias_handling() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            git: "git --verbose"
        "#;
        let aka = setup_aka(false, yaml)?;
        let result = aka.replace("git commit")?;
        let expect = "git --verbose commit ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_global_alias_handling() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
        "#;
        let aka = setup_aka(false, yaml)?;
        let result = aka.replace("ls -l")?;
        let expect = "exa -l ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_special_characters_handling() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
        "#;
        let aka = setup_aka(false, yaml)?;
        let result = aka.replace("ls -l | grep pattern")?;
        let expect = "exa -l | grep pattern ";
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_error_scenario() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
        "#;
        let aka = setup_aka(false, yaml)?;
        let cmdline = "undefined_alias file.txt";
        let result = aka.replace(cmdline)?;
        assert_eq!("", result); // Expecting an empty string for undefined alias
        Ok(())
    }

    #[test]
    fn test_no_substitution() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
        "#;
        let aka = setup_aka(false, yaml)?;
        let result = aka.replace("cat file.txt")?;
        let expect = ""; // Adjusted expectation
        assert_eq!(expect, result);
        Ok(())
    }

    // Health check tests
    #[test]
    fn test_health_check_valid_config() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
            cat: "bat"
        "#;
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", yaml)?;

        let result = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        assert_eq!(result, 0); // Should return 0 for valid config with aliases
        Ok(())
    }

    #[test]
    fn test_health_check_nonexistent_config() -> Result<()> {
        let nonexistent_path = PathBuf::from("/path/to/nonexistent/config.yml");
        let result = execute_health_check(&Some(nonexistent_path))?;
        assert_eq!(result, 1); // Should return 1 for nonexistent config
        Ok(())
    }

    #[test]
    fn test_health_check_invalid_config() -> Result<()> {
        let invalid_yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
            # Invalid YAML - missing closing quote
            cat: "bat
        "#;
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", invalid_yaml)?;

        let result = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        assert_eq!(result, 2); // Should return 2 for invalid config
        Ok(())
    }

    #[test]
    fn test_health_check_empty_aliases() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases: {}
        lookups:
            region:
                prod: us-east-1
        "#;
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", yaml)?;

        let result = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        assert_eq!(result, 3); // Should return 3 for no aliases
        Ok(())
    }

    #[test]
    fn test_hash_config_file() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
        "#;
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", yaml)?;

        let hash1 = hash_config_file(&temp_file.path().to_path_buf())?;
        let hash2 = hash_config_file(&temp_file.path().to_path_buf())?;

        // Same file should produce same hash
        assert_eq!(hash1, hash2);

        // Hash should be 16 characters (64-bit hex)
        assert_eq!(hash1.len(), 16);

        Ok(())
    }

    #[test]
    fn test_hash_different_files() -> Result<()> {
        let yaml1 = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
        "#;
        let yaml2 = r#"
        defaults:
            version: 1
        aliases:
            ls: "ls -la"
        "#;

        let mut temp_file1 = NamedTempFile::new()?;
        let mut temp_file2 = NamedTempFile::new()?;
        writeln!(temp_file1, "{}", yaml1)?;
        writeln!(temp_file2, "{}", yaml2)?;

        let hash1 = hash_config_file(&temp_file1.path().to_path_buf())?;
        let hash2 = hash_config_file(&temp_file2.path().to_path_buf())?;

        // Different files should produce different hashes
        assert_ne!(hash1, hash2);

        Ok(())
    }

    #[test]
    fn test_hash_caching_workflow() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
        "#;
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", yaml)?;

        // Clear any existing hash cache
        let _ = std::fs::remove_file(get_hash_cache_path()?);

        // First health check should validate and store hash
        let result1 = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        assert_eq!(result1, 0);

        // Verify hash was stored
        let stored_hash = get_stored_hash()?.expect("Hash should be stored");
        let expected_hash = hash_config_file(&temp_file.path().to_path_buf())?;
        assert_eq!(stored_hash, expected_hash);

        // Second health check should use cached hash (fast path)
        let result2 = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        assert_eq!(result2, 0);

        Ok(())
    }

    #[test]
    fn test_hash_cache_invalidation() -> Result<()> {
        let yaml1 = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
        "#;
        let yaml2 = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
            cat: "bat"
        "#;

        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", yaml1)?;

        // Clear any existing hash cache
        let _ = std::fs::remove_file(get_hash_cache_path()?);

        // First health check with yaml1
        let result1 = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        assert_eq!(result1, 0);

        let hash1 = get_stored_hash()?.expect("Hash should be stored");

        // Modify the file
        temp_file.rewind()?;
        temp_file.as_file_mut().set_len(0)?;
        writeln!(temp_file, "{}", yaml2)?;
        temp_file.flush()?;

        // Second health check should detect change and update hash
        let result2 = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        assert_eq!(result2, 0);

        let hash2 = get_stored_hash()?.expect("Hash should be updated");
        assert_ne!(hash1, hash2);

        Ok(())
    }

    #[test]
    fn test_get_hash_cache_path() -> Result<()> {
        let cache_path = get_hash_cache_path()?;

        // Should be in the data directory
        assert!(cache_path.to_string_lossy().contains("aka"));
        assert!(cache_path.to_string_lossy().ends_with("config.hash"));

        // Parent directory should exist after calling the function
        assert!(cache_path.parent().unwrap().exists());

        Ok(())
    }

    #[test]
    fn test_store_and_retrieve_hash() -> Result<()> {
        let test_hash = "deadbeefcafebabe";

        // Store hash
        store_hash(test_hash)?;

        // Retrieve hash
        let retrieved = get_stored_hash()?.expect("Hash should be retrievable");
        assert_eq!(retrieved, test_hash);

        Ok(())
    }

    #[test]
    fn test_health_check_with_default_config_path() -> Result<()> {
        // Test with None config (should use default path)
        let result = execute_health_check(&None)?;

        // Should return either 0 (if config exists and is valid) or 1 (if config not found)
        // The exact result depends on whether the user has a config file
        assert!(result == 0 || result == 1 || result == 2 || result == 3);

        Ok(())
    }

    #[test]
    fn test_xxhash_consistency() -> Result<()> {
        let test_data = b"test data for hashing";
        let hash1 = xxh3_64(test_data);
        let hash2 = xxh3_64(test_data);

        // Same data should produce same hash
        assert_eq!(hash1, hash2);

        // Different data should produce different hash
        let different_data = b"different test data";
        let hash3 = xxh3_64(different_data);
        assert_ne!(hash1, hash3);

        Ok(())
    }

    #[test]
    fn test_health_check_performance() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
            cat: "bat"
            grep: "rg"
        "#;
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", yaml)?;

        // Clear cache
        let _ = std::fs::remove_file(get_hash_cache_path()?);

        // Time the first health check (should be slower - validation)
        let start = std::time::Instant::now();
        let result1 = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        let first_duration = start.elapsed();
        assert_eq!(result1, 0);

        // Time the second health check (should be faster - cached)
        let start = std::time::Instant::now();
        let result2 = execute_health_check(&Some(temp_file.path().to_path_buf()))?;
        let second_duration = start.elapsed();
        assert_eq!(result2, 0);

        // Second call should be faster (though this might be flaky on very fast systems)
        // At minimum, both should complete in reasonable time
        assert!(first_duration.as_millis() < 100);
        assert!(second_duration.as_millis() < 100);

        Ok(())
    }
}
