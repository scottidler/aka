use anyhow::Result;
use anyhow::anyhow;
use log::{info, warn};
use std::env;

use std::path::{Path, PathBuf};
use structopt::StructOpt; //FIXME: consider parsing by hand

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

    cmdline: String,
}

fn test_path(config: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(file) = config {
        if file.exists() {
            return Some(file)
        }
    }
    None
}

fn divine_config() -> Option<PathBuf> {
    let configs: Vec<PathBuf> = CONFIGS.into_iter().map(|x| PathBuf::from(x)).collect();
    for config in configs {
        if config.exists() {
            return Some(PathBuf::from(config))
        }
    }
    None
}

fn main() -> Result<()> {
    let args = Args::from_args();

    println!("args: {:#?}", args);

    let verbose = true;
    if verbose {
        info!("args = {:?}", args);
        warn!("hi");
    }

    let filename = "aka.yml";
    let loader = Loader::new();
    //println!("loader: {:#?}", loader);

    let spec = loader.load(filename).unwrap();
    println!("spec: {:#?}", spec);

    Ok(())
}
