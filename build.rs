use std::process::Command;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=GIT_DESCRIBE");
    let git_describe = env::var("GIT_DESCRIBE").unwrap_or_else(|_| {
        if git_exists() && in_git_repo() {
            // Run `git describe` command
            let output = Command::new("git")
                .args(&["describe"])
                .output()
                .expect("Failed to execute `git describe`");

            // Get the output as a string
            String::from_utf8(output.stdout).expect("Not UTF-8")
        } else {
            String::from("unknown")
        }
    });

    // Write the output to a file
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("git_describe.rs");
    let mut f = File::create(&dest_path).unwrap();

    write!(f, "pub const GIT_DESCRIBE: &'static str = \"{}\";", git_describe).unwrap();
}

fn git_exists() -> bool {
    Command::new("git")
        .args(&["--version"])
        .output()
        .is_ok()
}

fn in_git_repo() -> bool {
    Command::new("git")
        .args(&["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| output.stdout == b"true\n")
        .unwrap_or(false)
}