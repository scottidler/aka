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

#[derive(Debug, StructOpt)]
#[structopt(name = "aka", about = "[a]lso [k]nown [a]s: an aliasing program")]
struct Args {
    #[structopt(short, long)]
    config: Option<PathBuf>,

    #[structopt(short, long, help = "is entry an [e]nd [o]f [l]ine?")]
    eol: bool,

    #[structopt(short, long)]
    verbose: bool,

    cmdline: String,
}

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

#[derive(Debug)]
struct AKA {
    pub spec: Spec,
    pub cmdline: String,
    pub eol: bool,
}

impl AKA {
    pub fn new(cmdline: &String, eol: bool, config: Option<PathBuf>) -> Result<Self> {
        let config = match &config {
            Some(file) => test_config(file)?,
            None => divine_config()?,
        };
        let loader = Loader::new();
        let spec = loader.load(&config).unwrap();
        Ok(Self {
            spec,
            cmdline: cmdline.to_owned(),
            eol,
        })
    }
    pub fn use_alias(alias: &Alias, pos: usize) -> bool {
        if pos == 0 {
            true
        }
        else if alias.global {
            true
        }
        else {
            false
        }
    }

    pub fn replace(&self) -> Result<String> {
        let mut args = split(&self.cmdline).unwrap_or(vec![]);
        let mut space = " ";
        let mut replaced = false;
        let mut pos: usize = 0;
        while pos < args.len() {
            let arg = &args[pos];
            let remainders: Vec<String> = args[pos+1..].to_vec();
            let value = match self.spec.aliases.get(arg) {
                Some(alias) if AKA::use_alias(&alias, pos) => {
                    replaced = true;
                    space = if alias.space { " " } else { "" };
                    let positionals = alias.positionals();
                    let _keywords = alias.keywords();
                    if positionals.len() == remainders.len() {
                        let mut result = alias.value.to_owned();
                        let zipped = positionals.iter().zip(remainders.iter());
                        for (positional, value) in zipped {
                            result = result.replace(positional, value);
                        }
                        args.drain(pos+1..pos+1+positionals.len());
                        result
                    }
                    else {
                        replaced = false;
                        arg.to_owned()
                    }
                },
                Some(_) => arg.to_owned(),
                None => arg.to_owned(),
            };
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
    let args = Args::from_args();
    let aka = AKA::new(&args.cmdline, args.eol, args.config)?;
    println!("aka.eol={:?}", aka.eol);
    let result = aka.replace()?;
    println!("{}", result);
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
