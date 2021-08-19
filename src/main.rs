use anyhow::{anyhow, Result};
use std::process::exit;
//use log::{info, warn};

use std::path::PathBuf;
use structopt::StructOpt; //FIXME: consider parsing by hand
use shellexpand::tilde;
use shlex::split;

pub mod cfg;
use cfg::loader::Loader;
use cfg::spec::Spec;
use cfg::alias::Alias;

const CONFIGS: &'static [&'static str] = &[
    "./aka.yml",
    "~/.aka.yml",
    "~/.config/aka/aka.yml",
];

fn divine_config() -> Result<PathBuf> {
    let configs: Vec<PathBuf> = CONFIGS
        .into_iter()
        .map(|file| tilde(file))
        .map(|file| PathBuf::from(file.as_ref()))
        .collect();
    for config in configs {
        if config.exists() {
            return Ok(config);
        }
    }
    Err(anyhow!("couldn't divine a config!"))
}

fn test_config(file: &PathBuf) -> Result<PathBuf> {
    if file.exists() {
        return Ok(file.to_owned())
    }
    Err(anyhow!("config {:?} not found!", file))
}

#[derive(Debug, StructOpt)]
#[structopt(name = "aka", about = "[a]lso [k]nown [a]s: an aliasing program")]
struct AkaOpts {
    #[structopt(short, long, help = "is entry an [e]nd [o]f [l]ine?")]
    eol: bool,

    #[structopt(short, long)]
    config: Option<PathBuf>,

    #[structopt(short = "v",  parse(from_occurrences))]
    verbosity: u8,

    // SUBCOMMANDS
    #[structopt(subcommand)]
    command: Option<Command>
}

#[derive(StructOpt, Debug)]
#[structopt(name = "comand", about = "choose command to run")]
enum Command {
    #[structopt(name = "ls", about = "list aka aliases")]
    List(ListOpts),

    #[structopt(name = "query", about = "query for aka substitutions")]
    Query(QueryOpts),
}

#[derive(Debug, StructOpt)]
struct QueryOpts {
    cmdline: String,
}

#[derive(Debug, StructOpt)]
struct ListOpts {
    params: Vec<String>
}

#[derive(Debug)]
struct AKA {
    pub eol: bool,
    pub spec: Spec,
}

impl AKA {
    pub fn new(eol: bool, config: Option<PathBuf>) -> Result<Self> {
        let config = match &config {
            Some(file) => test_config(file)?,
            None => divine_config()?,
        };
        let loader = Loader::new();
        let spec = loader.load(&config).unwrap();
        Ok(Self {
            spec,
            eol,
        })
    }
    pub fn use_alias(&self, alias: &Alias, pos: usize) -> bool {
        if alias.is_variadic() && !self.eol {
            false
        }
        else if pos == 0 {
            true
        }
        else if alias.global {
            true
        }
        else {
            false
        }
    }

    pub fn replace(&self, cmdline: &String) -> Result<String> {
        let mut pos: usize = 0;
        let mut space = " ";
        let mut replaced = false;
        let mut args = split(&cmdline).unwrap_or(vec![]);
        while pos < args.len() {
            let arg = &args[pos];
            let mut remainders: Vec<String> = args[pos+1..].to_vec();
            let (value, count) = match self.spec.aliases.get(arg) {
                Some(alias) if self.use_alias(&alias, pos) => {
                    replaced = true;
                    space = if alias.space { " " } else { "" };
                    let (v,c) = alias.replace(&mut remainders);
                    if v == alias.name {
                        replaced = false;
                    }
                    (v,c)
                },
                Some(_) => (arg.to_owned(), 0),
                None => (arg.to_owned(), 0),
            };
            let beg = pos+1;
            let end = beg+count;
            args.drain(beg..end);
            args[pos] = value;
            pos += 1;
        }
        if replaced {
            Ok(format!("{}{}", args.join(" "), space))
        }
        else {
            Ok("".to_owned())
        }
    }
}

fn execute() -> Result<i32> {
    let aka_opts = AkaOpts::from_args();
    let aka = AKA::new(aka_opts.eol, aka_opts.config)?;
    if let Some(command) = aka_opts.command {
        match command {
            Command::Query(query_opts) => {
                let result = aka.replace(&query_opts.cmdline)?;
                println!("{}", result);
            },
            Command::List(_list_opts) => {
                let mut aliases: Vec<Alias> = aka.spec.aliases.values().cloned().collect();
                aliases.sort_by_key(|a| a.name.to_owned() );
                for alias in aliases {
                    println!("{}: {}", alias.name, alias.value);
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
            eprintln!("error: {:?}", err);
            1
        }
    });
}
