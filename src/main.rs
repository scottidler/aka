use anyhow::Result;
use log::{info, warn};
use std::env;

pub mod cfg;
use cfg::loader::Loader;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
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
