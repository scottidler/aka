use anyhow::{Context, Error, Result};
use std::fs;
use std::path::PathBuf;

use super::spec::{Spec, Alias};

#[derive(Debug, Clone, PartialEq)]
pub struct Loader {}

impl Loader {
    pub fn new() -> Self {
        Self {}
    }

    pub fn load(&self, filename: &PathBuf) -> Result<Spec, Error> {
        let content =
            fs::read_to_string(filename).context(format!("Can't load filename={:?}", filename))?;
        let spec: Spec =
            serde_yaml::from_str(&content).context(format!("Can't load content={:?}", content))?;
        Ok(spec)
    }
}
