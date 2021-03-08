use anyhow::Result;
use anyhow::anyhow;
use log::{info, warn};
use std::env;
use std::process::exit;

use std::path::{Path, PathBuf};
use structopt::StructOpt; //FIXME: consider parsing by hand
use shellexpand::tilde;

pub mod cfg;
use cfg::loader::Loader;

const CONFIGS: &'static [&'static str] = &[
    "~/.config/aka/aka.yml",
    "~/.aka.yml",
    "./aka.yml",
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

fn aka() -> Result<i32> {
    let args = Args::from_args();
    println!("args = {:#?}", args);

    let config = match &args.config {
        Some(file) => test_config(file)?,
        None => divine_config()?,
    };
    if args.verbose {
        info!("args = {:?}", args);
        warn!("hi");
    }

    let loader = Loader::new();
    let spec = loader.load(&config).unwrap();
    println!("spec: {:#?}", spec);

    Ok(0)
}

fn main() {
    exit(match aka() {
        Ok(exitcode) => exitcode,
        Err(err) => {
            eprintln!("error: {:?}", err);
            1
        }
    });
}
