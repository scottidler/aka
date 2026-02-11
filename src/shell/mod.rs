//! Shell integration module for aka
//!
//! This module provides shell initialization scripts that are embedded into the binary
//! at compile time. The `.zsh` files in this directory are included via `include_str!()`
//! so they can be edited with proper syntax highlighting while still being distributed
//! as part of the single binary.
//!
//! This directory (rather than a flat `shell.rs`) exists to co-locate the `.zsh` asset
//! with the Rust code that embeds it — `include_str!("init.zsh")` resolves relative to
//! this file, keeping the script and its owner together without path gymnastics.
//!
//! # Usage
//!
//! Users add this to their `.zshrc`:
//! ```zsh
//! if hash aka 2>/dev/null; then
//!     eval "$(aka shell-init zsh)"
//! fi
//! ```

/// ZSH initialization script embedded at compile time
///
/// This script is loaded from `src/shell/init.zsh` and provides:
/// - Space key alias expansion while typing
/// - Accept-line alias expansion before execution
/// - Ctrl+t fuzzy alias search (requires sk or fzf)
/// - Killswitch support via ~/aka-killswitch file
/// - Completion integration
pub const ZSH_INIT_SCRIPT: &str = include_str!("init.zsh");

/// Generate shell initialization script for the specified shell
///
/// # Arguments
/// * `shell` - Shell type (currently only "zsh" is supported)
///
/// # Returns
/// * `Some(&str)` - The initialization script for the requested shell
/// * `None` - If the shell is not supported
///
/// # Example
/// ```
/// use aka_lib::shell::generate_init_script;
///
/// if let Some(script) = generate_init_script("zsh") {
///     print!("{}", script);
/// }
/// ```
pub fn generate_init_script(shell: &str) -> Option<&'static str> {
    match shell.to_lowercase().as_str() {
        "zsh" => Some(ZSH_INIT_SCRIPT),
        // Future: add bash, fish support
        // "bash" => Some(BASH_INIT_SCRIPT),
        // "fish" => Some(FISH_INIT_SCRIPT),
        _ => None,
    }
}

/// List of supported shells
pub fn supported_shells() -> &'static [&'static str] {
    &["zsh"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::const_is_empty)]
    fn test_zsh_init_script_not_empty() {
        // ZSH_INIT_SCRIPT is a const, so we verify it has content
        assert!(!ZSH_INIT_SCRIPT.is_empty());
    }

    #[test]
    fn test_zsh_init_script_contains_key_functions() {
        let script = ZSH_INIT_SCRIPT;

        // Must have killswitch function
        assert!(script.contains("_aka_killswitch"));

        // Must have space expansion
        assert!(script.contains("_aka_expand_space"));

        // Must have accept-line handler
        assert!(script.contains("_aka_accept_line"));

        // Must have search function
        assert!(script.contains("_aka_search"));
    }

    #[test]
    fn test_zsh_init_script_has_key_bindings() {
        let script = ZSH_INIT_SCRIPT;

        // Space key binding
        assert!(script.contains(r#"bindkey " " _aka_expand_space"#));

        // Ctrl+t binding for search
        assert!(script.contains(r#"bindkey "^t" _aka_search"#));
    }

    #[test]
    fn test_zsh_init_script_registers_widgets() {
        let script = ZSH_INIT_SCRIPT;

        assert!(script.contains("zle -N _aka_expand_space"));
        assert!(script.contains("zle -N _aka_accept_line"));
        assert!(script.contains("zle -N _aka_search"));
        assert!(script.contains("zle -N accept-line _aka_accept_line"));
    }

    #[test]
    fn test_zsh_init_script_has_completion_support() {
        let script = ZSH_INIT_SCRIPT;

        assert!(script.contains("_aka_complete_commands"));
        assert!(script.contains("aka __complete_aliases"));
        assert!(script.contains("compdef"));
    }

    #[test]
    fn test_generate_init_script_zsh() {
        let script = generate_init_script("zsh");
        assert!(script.is_some());
        assert_eq!(script.unwrap(), ZSH_INIT_SCRIPT);
    }

    #[test]
    fn test_generate_init_script_zsh_case_insensitive() {
        assert!(generate_init_script("ZSH").is_some());
        assert!(generate_init_script("Zsh").is_some());
        assert!(generate_init_script("ZsH").is_some());
    }

    #[test]
    fn test_generate_init_script_unsupported() {
        assert!(generate_init_script("bash").is_none());
        assert!(generate_init_script("fish").is_none());
        assert!(generate_init_script("").is_none());
        assert!(generate_init_script("invalid").is_none());
    }

    #[test]
    fn test_supported_shells() {
        let shells = supported_shells();
        assert!(!shells.is_empty());
        assert!(shells.contains(&"zsh"));
    }

    #[test]
    fn test_zsh_init_script_has_header_comment() {
        let script = ZSH_INIT_SCRIPT;

        // Should have usage instructions
        assert!(script.contains("eval \"$(aka shell-init zsh)\""));
    }

    #[test]
    fn test_zsh_init_script_killswitch_path() {
        let script = ZSH_INIT_SCRIPT;

        // Should reference the killswitch file
        assert!(script.contains("~/aka-killswitch"));
    }

    #[test]
    fn test_zsh_init_script_calls_aka_binary() {
        let script = ZSH_INIT_SCRIPT;

        // Should call aka query
        assert!(script.contains("aka query"));

        // Should call aka --eol query
        assert!(script.contains("aka --eol query"));

        // Should call aka ls for search
        assert!(script.contains("aka ls"));
    }

    // -------------------------------------------------------------------------
    // Visual artifact prevention tests
    //
    // These tests enforce that both the Space and Enter handlers use the same
    // pattern to prevent stale zsh-syntax-highlighting from ghosting onto the
    // expanded command text. The fix requires:
    //   1. POSTDISPLAY="" BEFORE BUFFER assignment (clear ghost text first)
    //   2. zle reset-prompt AFTER BUFFER/CURSOR assignment (force full redraw)
    //
    // Without these, ZSH's region_highlight array retains stale color ranges
    // from the short alias text and applies them to the expanded command.
    // -------------------------------------------------------------------------

    /// Helper: extract the body of a ZSH function from the init script.
    /// Returns the text between the opening `{` and the matching closing `}`.
    fn extract_zsh_function(script: &str, name: &str) -> String {
        let pattern = format!("{name}()");
        let start = script
            .find(&pattern)
            .unwrap_or_else(|| panic!("function {name}() not found in init.zsh"));
        let after_sig = &script[start..];
        let brace_start = after_sig
            .find('{')
            .unwrap_or_else(|| panic!("opening brace not found for {name}()"));
        let body_start = start + brace_start + 1;

        // Walk forward counting braces to find the matching close
        let mut depth = 1u32;
        let mut end = body_start;
        for (i, ch) in script[body_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = body_start + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        script[body_start..end].to_string()
    }

    #[test]
    fn test_accept_line_has_reset_prompt() {
        // The Enter handler MUST call `zle reset-prompt` after modifying BUFFER
        // to force ZSH to fully re-render, clearing stale syntax highlighting.
        // This was the root cause of the red-highlight ghost artifact bug.
        let body = extract_zsh_function(ZSH_INIT_SCRIPT, "_aka_accept_line");
        assert!(
            body.contains("zle reset-prompt"),
            "_aka_accept_line must call `zle reset-prompt` to clear stale \
             syntax highlighting after alias expansion. Without this, \
             region_highlight entries from the original alias text ghost \
             onto the expanded command.\n\nFunction body:\n{body}"
        );
    }

    #[test]
    fn test_expand_space_has_reset_prompt() {
        // The Space handler must also have `zle reset-prompt` (regression guard)
        let body = extract_zsh_function(ZSH_INIT_SCRIPT, "_aka_expand_space");
        assert!(
            body.contains("zle reset-prompt"),
            "_aka_expand_space must call `zle reset-prompt`.\n\nFunction body:\n{body}"
        );
    }

    #[test]
    fn test_accept_line_clears_postdisplay_before_buffer() {
        // POSTDISPLAY="" must come BEFORE BUFFER= to prevent autosuggestion
        // ghost text from blending with the new buffer during redraw.
        let body = extract_zsh_function(ZSH_INIT_SCRIPT, "_aka_accept_line");
        let postdisplay_pos = body
            .find("POSTDISPLAY=\"\"")
            .unwrap_or_else(|| panic!("_aka_accept_line must clear POSTDISPLAY.\n\nFunction body:\n{body}"));
        let buffer_pos = body
            .find("BUFFER=\"$output\"")
            .unwrap_or_else(|| panic!("_aka_accept_line must set BUFFER.\n\nFunction body:\n{body}"));
        assert!(
            postdisplay_pos < buffer_pos,
            "_aka_accept_line must clear POSTDISPLAY before setting BUFFER \
             to prevent ghost text artifacts during redraw.\n\
             POSTDISPLAY at byte {postdisplay_pos}, BUFFER at byte {buffer_pos}\n\n\
             Function body:\n{body}"
        );
    }

    #[test]
    fn test_expand_space_clears_postdisplay_before_buffer() {
        // Same ordering requirement for the Space handler (regression guard)
        let body = extract_zsh_function(ZSH_INIT_SCRIPT, "_aka_expand_space");
        let postdisplay_pos = body
            .find("POSTDISPLAY=\"\"")
            .unwrap_or_else(|| panic!("_aka_expand_space must clear POSTDISPLAY.\n\nFunction body:\n{body}"));
        let buffer_pos = body
            .find("BUFFER=\"$output\"")
            .unwrap_or_else(|| panic!("_aka_expand_space must set BUFFER.\n\nFunction body:\n{body}"));
        assert!(
            postdisplay_pos < buffer_pos,
            "_aka_expand_space must clear POSTDISPLAY before setting BUFFER.\n\
             POSTDISPLAY at byte {postdisplay_pos}, BUFFER at byte {buffer_pos}\n\n\
             Function body:\n{body}"
        );
    }

    #[test]
    fn test_both_handlers_have_consistent_artifact_prevention() {
        // Both handlers must use the same pattern for visual artifact prevention.
        // This test ensures they stay in sync and neither regresses independently.
        let space_body = extract_zsh_function(ZSH_INIT_SCRIPT, "_aka_expand_space");
        let enter_body = extract_zsh_function(ZSH_INIT_SCRIPT, "_aka_accept_line");

        let required_elements = [
            ("POSTDISPLAY=\"\"", "clear autosuggestion ghost text"),
            ("BUFFER=\"$output\"", "set buffer to expanded alias"),
            ("CURSOR=${#BUFFER}", "move cursor to end of expanded text"),
            (
                "zle reset-prompt",
                "force full prompt redraw to clear stale highlighting",
            ),
        ];

        for (element, purpose) in &required_elements {
            assert!(
                space_body.contains(element),
                "_aka_expand_space is missing `{element}` ({purpose})\n\nBody:\n{space_body}"
            );
            assert!(
                enter_body.contains(element),
                "_aka_accept_line is missing `{element}` ({purpose})\n\nBody:\n{enter_body}"
            );
        }

        // Verify ordering: POSTDISPLAY < BUFFER < CURSOR < reset-prompt
        for (body, name) in [(&space_body, "_aka_expand_space"), (&enter_body, "_aka_accept_line")] {
            let positions: Vec<(&str, usize)> = required_elements
                .iter()
                .map(|(elem, _)| {
                    let pos = body.find(elem).unwrap();
                    (*elem, pos)
                })
                .collect();

            for window in positions.windows(2) {
                assert!(
                    window[0].1 < window[1].1,
                    "In {name}: `{}` (byte {}) must come before `{}` (byte {})\n\nBody:\n{body}",
                    window[0].0,
                    window[0].1,
                    window[1].0,
                    window[1].1,
                );
            }
        }
    }
}
