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
    pub args: Vec<String>,
    pub spec: Spec,
}

impl AKA {
    pub fn new(cmdline: String, config: Option<PathBuf>) -> Result<Self> {
        let args = split(&cmdline)
            .ok_or(anyhow!("failed to split cmdline={:?}", cmdline))?;
        let config = match &config {
            Some(file) => test_config(file)?,
            None => divine_config()?,
        };
        let loader = Loader::new();
        let spec = loader.load(&config).unwrap();
        Ok(Self {
            args,
            spec,
        })
    }

    pub fn replace(&mut self) -> Result<i32> {
        let mut i: usize = 0;
        let mut args: Vec<String> = vec![];
        while i < self.args.len() {
            let arg = &self.args[i];
            let _rem: Vec<String> = self.args[i+i..].to_vec();
            let value = match self.spec.aliases.get(arg) {
                Some(alias) => {
                    if alias.first {
                        if i == 0 {
                            alias.value.clone()
                        }
                        else {
                            arg.clone()
                        }
                    }
                    else {
                        alias.value.clone()
                    }
                },
                None => arg.clone(),
            };
            args.push(value);
            i += 1;
        }
        self.args = args;
        Ok(0)
    }
}

fn execute() -> Result<i32> {
    let args = Args::from_args();
    let mut aka = AKA::new(args.cmdline, args.config)?;
    let result = aka.replace();
    println!("{}", aka.args.join(" "));
    result
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
