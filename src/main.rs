use clap::Parser;
use eyre::{eyre, Result};
use shellexpand::tilde;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::exit;

pub mod cfg;
use cfg::alias::Alias;
use cfg::loader::Loader;
use cfg::spec::Spec;

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

    pub fn replace(&self, cmdline: &str) -> Result<Vec<String>> {
        let mut pos: usize = 0;
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
            let arg = &args[pos];
            let mut remainders: Vec<String> = args[pos + 1..].to_vec();
            let (value, count) = match self.spec.aliases.get(arg) {
                Some(alias) if self.use_alias(alias, pos) => {
                    let (v, c) = alias.replace(&mut remainders)?;
                    if v != alias.name {
                        replaced = true;
                    }
                    (v, c)
                }
                Some(_) | None => (arg.clone(), 0),
            };

            let beg = pos + 1;
            let end = beg + count;
            args.drain(beg..end);
            args.splice(pos..=pos, Self::split_respecting_quotes(&value));
            pos += 1;
        }

        if sudo {
            args[0] = format!("$(which {})", args[0]);
            args.insert(0, "sudo".to_string());
        }

        Ok(if replaced || sudo { args } else { Vec::new() })
    }
}

fn print_alias(alias: &Alias) {
    if alias.value.contains('\n') {
        println!("{}: |\n  {}", alias.name, alias.value.replace("\n", "\n  "));
    } else {
        println!("{}: {}", alias.name, alias.value);
    }
}

fn execute() -> Result<i32> {
    let aka_opts = AkaOpts::parse();
    let aka = AKA::new(aka_opts.eol, &aka_opts.config)?;
    if let Some(command) = aka_opts.command {
        match command {
            Command::Query(query_opts) => {
                let result = aka.replace(&query_opts.cmdline)?;
                if std::env::var("AKA_LOG").is_ok() {
                    let mut file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .append(true)
                        .open("/home/saidler/aka.log")?;
                    writeln!(file, "'{}' -> '{}'", query_opts.cmdline, result.join(" "))?;
                }
                println!("{}", result.join(" "));
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
    exit(match execute() {
        Ok(exitcode) => exitcode,
        Err(err) => {
            eprintln!("error: {err:?}");
            1
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eyre::Result;
    use pretty_assertions::assert_eq;
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
        let expect = vec!["bat", "-p", "file.txt"];
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
        let expect = vec!["bat", "-p", "file.txt", "|", "xclip", "-sel", "clip", "&&", "echo", "test"];
        assert_eq!(expect, result);
        Ok(())
    }

    #[test]
    fn test_exclamation_mark_at_end() -> Result<()> {
        let yaml = r#"
        defaults:
            version: 1
        aliases:
            vim: "nvim"
        "#;
        let aka = setup_aka(true, yaml)?;
        let result = aka.replace("vim file.txt !")?;
        let expect = vec!["sudo", "$(which nvim)", "file.txt"];
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
        let expect = vec!["bat", "-p", "/some/file"];
        assert_eq!(expect, result);
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
        let expect: Vec<String> = vec![];
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
        let expect = vec!["git", "--verbose", "commit"];
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
        let expect = vec!["exa", "-l"];
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
        let expect = vec!["exa", "-l", "|", "grep", "pattern"];
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
        let expect: Vec<String> = vec![];
        assert_eq!(expect, result);
        Ok(())
    }
}
