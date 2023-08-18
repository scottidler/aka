use std::env;
use std::fs::{read_to_string, write, File};
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn git_describe_value() -> String {
    env::var("GIT_DESCRIBE").unwrap_or_else(|_| {
        let output = Command::new("git")
            .args(&["describe"])
            .output()
            .expect("Failed to execute `git describe`");

        String::from_utf8(output.stdout).expect("Not UTF-8")
    })
}

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let git_describe = Path::new(&out_dir).join("GIT_DESCRIBE");
    println!("BUILD_RS: GIT_DESCRIBE file is located at: {}", git_describe.display());

    let old_value = read_to_string(&git_describe).unwrap_or_default();

    let new_value = git_describe_value();

    if new_value != old_value {
        println!("BUILD_RS: old_value='{old_value}' != new_value='{new_value}'");

        write(&git_describe, &new_value).unwrap();

        let git_describe_rs = Path::new(&out_dir).join("git_describe.rs");
        let mut f = File::create(&git_describe_rs).unwrap();

        write!(f, "pub const GIT_DESCRIBE: &'static str = \"{}\";", new_value).unwrap();

        println!("cargo:rerun-if-changed={}", git_describe.display());
    } else {
        println!("BUILD_RS: old_value='{old_value}' == new_value='{new_value}'");
    }
}
