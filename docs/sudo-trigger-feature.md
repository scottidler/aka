# Sudo Trigger Feature (`!` → `sudo`)

**Status**: ✅ Recovered and Implemented  
**Date**: January 2025  
**Version**: v0.5.0+

## Overview

The sudo trigger feature allows users to append `!` to the end of any command to automatically prefix it with `sudo`. This provides a convenient way to re-run commands with elevated privileges without retyping the entire command.

## User Experience

### Basic Usage
```bash
# User types a command that fails due to permissions
❯ touch /etc/somefile
touch: cannot touch '/etc/somefile': Permission denied

# User can then add ! to retry with sudo
❯ touch /etc/somefile !
# Transforms to: sudo touch /etc/somefile
```

### With Alias Expansion
```bash
# If 'ls' is aliased to 'eza'
❯ ls /root !
# Transforms to: sudo -E $(which eza) /root
```

### Complex Commands
```bash
❯ systemctl restart nginx !
# Transforms to: sudo systemctl restart nginx
```

## Technical Requirements

### Core Behavior
1. **Only works on Enter press** (`eol=true`), not on space press
2. **Strips the `!`** from the final command
3. **Integrates with existing sudo logic**: Applies `$(which)` wrapping and `-E` flag as needed
4. **Ignores `!` in the middle** of commands: `echo hello ! world` should NOT trigger sudo
5. **Works with quoted arguments**: `echo "test" !` → `sudo echo "test"`

### Implementation Details
- **Detection**: `split_respecting_quotes()` function detects trailing `!` at end of command
- **Processing**: `replace_with_mode()` function removes `!` and sets `sudo = true`
- **Integration**: Uses existing sudo wrapping logic for `$(which)` and `-E` flag handling

## Code Architecture

### Key Components

#### 1. Command Parsing (`split_respecting_quotes`)
```rust
// Detects trailing ! and splits it as separate argument
else if chars[index] == '!' && !in_quotes && index == chars.len() - 1 {
    if start != index {
        args.push(cmdline[start..index].to_string());
    }
    args.push(String::from("!"));
    start = index + 1;
}
```

#### 2. Sudo Trigger Detection (`replace_with_mode`)
```rust
// Check for sudo trigger pattern: command ends with "!" (only when eol=true)
if self.eol && !args.is_empty() {
    if let Some(last_arg) = args.last() {
        if last_arg == "!" {
            args.pop(); // Remove the "!"
            sudo = true;
            // ... rest of processing
        }
    }
}
```

#### 3. Integration with Existing Sudo Logic
The feature reuses all existing sudo processing:
- `$(which)` wrapping for user-installed tools
- `-E` flag for environment preservation
- Argument handling and space management

## Test Coverage

### Comprehensive Test Suite
- **`test_sudo_trigger_comprehensive`**: Basic functionality and eol requirement
- **`test_sudo_trigger_edge_cases`**: Mid-command `!`, quotes, multiple `!`
- **`test_split_respecting_quotes_with_exclamation`**: Parsing logic validation

### Test Cases Covered
```rust
// Basic functionality
"touch file !" → "sudo touch file "

// Alias expansion
"ls !" → "sudo -E $(which eza) " // if ls → eza

// EOL requirement
// eol=true: "touch file !" → "sudo touch file "
// eol=false: "touch file !" → "" (no transformation)

// Edge cases
"echo hello ! world" → "echo 'hello world' hello ! world " // no sudo
"echo \"test\" !" → "sudo echo 'hello world' \"test\" " // with quotes
"!" → "" // lone exclamation ignored
```

## Historical Context

### Original Implementation
- **Commit**: `0abdfdd` (May 27, 2023)
- **Message**: "implemented ! -> sudo"
- **Location**: `src/main.rs`
- **Size**: 98 additions, 11 deletions

### Loss and Recovery
- **Lost During**: Code restructuring when moving from `src/main.rs` to `src/lib.rs`
- **Recovery Date**: January 2025
- **Recovery Method**: Found original implementation via git archaeology (`git log -S "!"`)

### Lessons Learned
- **Need for regression tests**: Feature was lost due to lack of comprehensive tests
- **Architecture documentation**: Better documentation prevents feature loss during refactoring

## Configuration

### No Configuration Required
The feature is always available when:
- `eol=true` (Enter press mode)
- Command ends with `!`
- Not inside quotes

### Integration with Existing Features
- **Works with all aliases**: Global and local
- **Works with lookups**: `lookup:region[prod] !` → `sudo us-east-1`
- **Works with variadic aliases**: Expands arguments correctly

## Future Considerations

### Potential Enhancements
1. **History integration**: Could integrate with shell history for `!!` style commands
2. **Configuration option**: Could make the trigger character configurable
3. **Multiple triggers**: Could support `!!`, `!sudo`, etc.

### Maintenance Notes
- **Critical tests**: Never remove `test_sudo_trigger_*` tests
- **Regression prevention**: Any refactoring must maintain test coverage
- **Documentation**: Keep this document updated with any changes

## Related Files

### Core Implementation
- `src/lib.rs`: `split_respecting_quotes()`, `replace_with_mode()`
- `src/bin/aka.rs`: Command routing and processing

### Tests
- `src/lib.rs`: `test_sudo_trigger_comprehensive`, `test_sudo_trigger_edge_cases`
- `src/lib.rs`: `test_split_respecting_quotes_with_exclamation`

### Documentation
- `README.md`: User-facing feature description
- `docs/sudo-trigger-feature.md`: This technical document 