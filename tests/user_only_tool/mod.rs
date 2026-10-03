/// The `$(which <tool>)` expectations in the tests that use this only hold where the tool is on the user's PATH
/// but not root's (a dev box with cargo-installed tools). On a bare CI runner they are
/// absent, so aka correctly leaves the command unwrapped and these cases do not apply.
pub fn is_user_only_tool(tool: &str) -> bool {
    let quiet = std::process::Stdio::null;
    let user_has = std::process::Command::new("which")
        .arg(tool)
        .stdout(quiet())
        .stderr(quiet())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let root_has = std::process::Command::new("sudo")
        .args(["-n", "which", tool])
        .stdout(quiet())
        .stderr(quiet())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    user_has && !root_has
}

macro_rules! require_user_only_tools {
    ($($tool:expr),+) => {
        $(
            if !$crate::user_only_tool::is_user_only_tool($tool) {
                eprintln!("skipping: {} is not a user-only tool on this machine", $tool);
                return;
            }
        )+
    };
}
