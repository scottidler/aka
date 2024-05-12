#![cfg_attr(debug_assertions, allow(unused_imports, unused_variables, unused_mut, dead_code))]

use clap::Parser;
use eyre::{eyre, Result};
use shellexpand::tilde;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::exit;
use log::{info, debug, warn, error};
use shlex::split;

pub mod cfg;
use cfg::alias::Alias;
use cfg::loader::Loader;
use cfg::spec::Spec;

#[macro_use]
mod macros;

const CONFIGS: &[&str] = &["./aka.yml", "~/.aka.yml", "~/.config/aka/aka.yml"];

fn divine_config() -> Result<PathBuf> {
    let configs: Vec<PathBuf> = CONFIGS
        .iter()
        .map(tilde)
        .map(|file| PathBuf::from(file.as_ref()))
        .collect();
    for config in configs {
        if config.exists() {
            return Ok(config);
        }
    }
    Err(eyre!("couldn't divine a config!"))
}

fn test_config(file: &PathBuf) -> Result<PathBuf> {
    if file.exists() {
        return Ok(file.clone());
    }
    Err(eyre!("config {:?} not found!", file))
}

mod built_info {
    include!(concat!(env!("OUT_DIR"), "/git_describe.rs"));
}

#[derive(Parser)]
#[command(name = "aka", about = "[a]lso [k]nown [a]s: an aliasing program")]
#[command(version = built_info::GIT_DESCRIBE)]
#[command(author = "Scott A. Idler <scott.a.idler@gmail.com>")]
#[command(arg_required_else_help = true)]
#[command(after_help = "set env var AKA_LOG to turn on logging to ~/aka.log")]
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
            None => divine_config()?,
        };
        let loader = Loader::new();
        let spec = loader.load(&config)?;
        Ok(Self { eol, spec })
    }

    pub fn use_alias(&self, alias: &Alias, pos: usize) -> bool {
        if alias.is_variadic() && !self.eol {
            false
        }
        else if pos == 0 {
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

    pub fn replace2(&self, mut cmdline: Vec<String>) -> Result<Vec<String>> {
        debug!("AKA::replace2: Start - cmdline={:?}", cmdline);
        let mut pos: usize = 0;
        let mut space = true;
        let mut sudo = false;
        let mut replaced = false;

        if self.eol && !cmdline.is_empty() {
            debug!("1 Handling EOL - Initial cmdline={:?}", cmdline);
            if let Some(last_arg) = cmdline.last().cloned() {
                debug!("2 AKA: Handling EOL - Last Argument={}", last_arg);
                if last_arg == "!" || last_arg.ends_with("!") {
                    cmdline.pop();
                    sudo = true;
                    debug!("3 AKA: Sudo Flag Set - cmdline after pop={:?}, sudo={}", cmdline, sudo);
                } else if last_arg.starts_with("!") {
                    let next_arg = last_arg[1..].to_string();
                    cmdline[0] = next_arg.clone();
                    replaced = true;
                    let mut i = 1;
                    debug!("4 AKA: Starts with '!' - next_arg={}, modified cmdline[0]={}", next_arg, cmdline[0]);
                    while i < cmdline.len() {
                        if cmdline[i].starts_with("-") {
                            cmdline.remove(i);
                            debug!("5 AKA: Removing '-' argument - Current cmdline={:?}", cmdline);
                        } else if cmdline[i] == "|" || cmdline[i] == ">" || cmdline[i] == "<" {
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    cmdline.pop();
                    debug!("6 AKA: Post-'!' Processing - Final cmdline={:?}", cmdline);
                }
            }
        }

        while pos < cmdline.len() {
            let arg = &cmdline[pos].clone();
            let remainders = cmdline[pos + 1..].to_vec();
            let (values, count) = match self.spec.aliases.get(arg) {
                Some(alias) if self.use_alias(alias, pos) => {
                    let (v, c) = alias.replace2(&mut remainders.clone())?;
                    if v != vec![alias.name.clone()] {
                        replaced = true;
                    }
                    (v, c)
                },
                Some(_) | None => (vec![arg.clone()], 0),
            };

            cmdline.splice(pos..pos + 1 + count, values.iter().cloned());
            pos += values.len();
        }
        if sudo {
            let command = format!("$(which {})", cmdline.remove(0));
            cmdline.insert(0, command);
            cmdline.insert(0, "sudo".to_string());
            debug!("Sudo command prepended - Final cmdline={:?}", cmdline);
        }
        if replaced || sudo {
            debug!("18 replace={} sudo={}", replaced, sudo);
            if space {
                debug!("19 space={}", space);
                cmdline.push("".to_string());
            }
            debug!("20 Result after processing replaced or sudo: {:?}", cmdline);
        } else {
            cmdline.clear();
            debug!("21 No replaced={} or sudo={} adjustments made, returning empty vector.", replaced, sudo);
        }
        Ok(cmdline)
    }
}

fn print_alias(alias: &Alias) {
    // FIXME: is most certainly not correct; but good enough for now
    println!("{}: {:?}", alias.name, alias.value);
}

fn execute() -> Result<i32> {
    let aka_opts = AkaOpts::parse();
    let aka = AKA::new(aka_opts.eol, &aka_opts.config)?;
    if let Some(command) = aka_opts.command {
        match command {
            Command::Query(query_opts) => {
                let args = split(&query_opts.cmdline).ok_or(eyre!("something"))?;
                let result = aka.replace2(args)?;
                let result = result.join(" ");
                if std::env::var("AKA_LOG").is_ok() {
                    let mut file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .append(true)
                        .open("/home/saidler/aka.log")?;
                    writeln!(file, "'{}' -> '{}'", query_opts.cmdline, result)?;
                }
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
        }
    }
    Ok(0)
}

fn main() {
    env_logger::init();
    info!("aka logger setup");
    exit(match execute() {
        Ok(exitcode) => exitcode,
        Err(err) => {
            eprintln!("error: {err:?}");
            1
        }
    });
}

#[cfg(test)]
mod tests2 {
    use super::*;
    use std::collections::HashMap;
    use eyre::{Result, WrapErr};
    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;
    use std::sync::Once;
    use log::LevelFilter;

    static INIT: Once = Once::new();
    pub fn initialize_logging() {
        INIT.call_once(|| {
            let _ = env_logger::builder().is_test(true).filter_level(LevelFilter::Debug).try_init();
        });
    }

    fn setup_aka2(eol: bool, yaml: &str) -> Result<AKA> {
        initialize_logging();
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "{}", yaml)?;
        let aka = AKA::new(eol, &Some(temp_file.path().to_path_buf()))?;
        Ok(aka)
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
    fn test_simple_substitution2() -> Result<()> {
        initialize_logging();
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
        "#;
        let aka = setup_aka2(false, yaml)?;
        let cmdline = vos!["cat", "file.txt"];
        let result = aka.replace2(cmdline)?;
        let expect = vos!["bat", "-p", "file.txt", ""];
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_exclamation_mark_handling2() -> Result<()> {
        initialize_logging();
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            vim: "nvim"
        "#;
        let aka = setup_aka2(true, yaml)?;
        let cmdline = vos!["vim", "file.txt", "!"];
        let result = aka.replace2(cmdline)?;
        let expect = vos!["sudo", "$(which nvim)", "file.txt", ""];
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_no_exclamation_mark2() -> Result<()> {
        initialize_logging();
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
        "#;
        let aka = setup_aka2(false, yaml)?;
        let cmdline = vos!["cat", "/some/file"];
        let result = aka.replace2(cmdline)?;
        let expect = vos!["bat", "-p", "/some/file", ""];
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_variadic_alias_handling2() -> Result<()> {
        initialize_logging();
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            git: "git --verbose"
        "#;
        let aka = setup_aka2(false, yaml)?;
        let cmdline = vos!["git", "commit"];
        let result = aka.replace2(cmdline)?;
        let expect = vos!["git", "--verbose", "commit", ""];
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_global_alias_handling2() -> Result<()> {
        initialize_logging();
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            ls: "exa"
        "#;
        let aka = setup_aka2(false, yaml)?;
        let cmdline = vos!["ls", "-l"];
        let result = aka.replace2(cmdline)?;
        let expect = vos!["exa", "-l", ""];
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_error_scenario2() -> Result<()> {
        initialize_logging();
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            cat: "bat -p"
        "#;
        let aka = setup_aka2(false, yaml)?;
        let cmdline = vos!["undefined_alias", "file.txt"];
        let result = aka.replace2(cmdline)?;
        let expect = Vec::<String>::new();
        assert_eq!(expect, result);
        Ok(())
    }
}
