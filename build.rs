use std::process::Command;
use std::fs::{File, write};
use std::io::Write;
use std::path::Path;
use std::env;

fn main() {
    // Get the output of `git describe` or the GIT_DESCRIBE environment variable
    let value = env::var("GIT_DESCRIBE").unwrap_or_else(|_| {
        let output = Command::new("git")
            .args(&["describe"])
            .output()
            .expect("Failed to execute `git describe`");

        String::from_utf8(output.stdout).expect("Not UTF-8")
    });

    // Write the output to a file
    let out_dir = env::var("OUT_DIR").unwrap();
    let git_describe_rs = Path::new(&out_dir).join("git_describe.rs");
    let mut f = File::create(&git_describe_rs).unwrap();

    write!(f, "pub const GIT_DESCRIBE: &'static str = \"{}\";", value).unwrap();

    // Write the GIT_DESCRIBE environment variable to a GIT_DESCRIBE file
    let git_describe = Path::new(&out_dir).join("GIT_DESCRIBE");
    write(&git_describe, &value).unwrap();

    // Tell Cargo to rerun the build script if the .git/HEAD file or the GIT_DESCRIBE file changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed={}", git_describe.display());
}