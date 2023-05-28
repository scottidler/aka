//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use eyre::{eyre, Result};
use std::process::exit;
use std::path::PathBuf;
use shellexpand::tilde;
use std::fs::OpenOptions;
use std::io::Write;
use clap::Parser;

pub mod cfg;
use cfg::loader::Loader;
use cfg::spec::Spec;
use cfg::alias::Alias;

const CONFIGS: &[&str] = &[
    "./aka.yml",
    "~/.aka.yml",
    "~/.config/aka/aka.yml",
];

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
        return Ok(file.clone())
    }
    Err(eyre!("config {:?} not found!", file))
}

#[derive(Parser)]
#[command(name = "aka", about = "[a]lso [k]nown [a]s: an aliasing program")]
#[command(version = "0.1.0")]
#[command(author = "Scott A. Idler <scott.a.idler@gmail.com>")]
#[command(arg_required_else_help = true)]
#[command(after_help = "set env var AKA_LOG to turn on logging to ~/aka.txt")]
struct AkaOpts {
    #[clap(short, long, help = "is entry an [e]nd [o]f [l]ine?")]
    eol: bool,

    #[clap(short, long)]
    config: Option<PathBuf>,

    // SUBCOMMANDS
    #[clap(subcommand)]
    command: Option<Command>
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
    patterns: Vec<String>
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
        }
        else {
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

    pub fn replace(&self, cmdline: &str) -> Result<String> {
        let mut pos: usize = 0;
        let mut space = " ";
        let mut replaced = false;
        let mut sudo = false;
        let mut args = Self::split_respecting_quotes(cmdline);

        // Check if last arg ends with '! ' and eol is true.
        if self.eol && !args.is_empty() {
            if let Some(last_arg) = args.last() {
                if last_arg == "!" {
                    args.pop(); // Remove '!'
                    sudo = true;
                }
            }
        }

        while pos < args.len() {
            let arg = &args[pos];
            let mut remainders: Vec<String> = args[pos+1..].to_vec();
            let (value, count) = match self.spec.aliases.get(arg) {
                Some(alias) if self.use_alias(alias, pos) => {
                    replaced = true;
                    space = if alias.space { " " } else { "" };
                    let (v,c) = alias.replace(&mut remainders)?;
                    if v == alias.name {
                        replaced = false;
                    }
                    (v,c)
                },
                Some(_) | None => (arg.clone(), 0),
            };

            let beg = pos+1;
            let end = beg+count;
            args.drain(beg..end);
            args.splice(pos..=pos, Self::split_respecting_quotes(&value));
            pos += 1;
        }

        if sudo {
            // Wrap the first argument's binary name in `$(which arg)`
            args[0] = format!("$(which {})", args[0]);
            args.insert(0, "sudo".to_string()); // Insert sudo at the beginning
        }

        let result = if replaced || sudo {
            format!("{}{}", args.join(" "), space)
        } else {
            String::new()
        };

        Ok(result)
    }

}

fn execute() -> Result<i32> {
    let aka_opts = AkaOpts::parse();
    let aka = AKA::new(aka_opts.eol, &aka_opts.config)?;
    if let Some(command) = aka_opts.command {
        match command {
            Command::Query(query_opts) => {
                let result = aka.replace(&query_opts.cmdline)?;
                // check for the existence of the AKA_LOG environment variable
                if std::env::var("AKA_LOG").is_ok() {
                    let mut file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .append(true)
                        .open("/home/saidler/aka.txt")?;
                    writeln!(file, "'{}' -> '{}'", query_opts.cmdline, result)?;
                }
                println!("{result}");
            },
            Command::List(list_opts) => {
                let mut aliases: Vec<Alias> = aka.spec.aliases.values().cloned().collect();
                aliases.sort_by_key(|a| a.name.clone());
                if list_opts.patterns.is_empty() {
                    for alias in aliases {
                        println!("{}: {}", alias.name, alias.value);
                    }
                } else {
                    for alias in aliases {
                        if list_opts.patterns.iter().any(|pattern| alias.name.starts_with(pattern)) {
                            println!("{}: {}", alias.name, alias.value);
                        }
                    }
                }
            },
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
