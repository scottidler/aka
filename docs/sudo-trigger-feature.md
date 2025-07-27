# Sudo Trigger Feature (`!` → `sudo`)

**Status**: ✅ Recovered, Implemented, and Tested
**Date**: January 2025
**Version**: v0.5.0+
**Resolution**: Shell interference issue identified and documented

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

#### Original Loss (2023)
- **Lost During**: Code restructuring when moving from `src/main.rs` to `src/lib.rs`
- **Original Implementation**: Commit `0abdfdd` (May 27, 2023) - "implemented ! -> sudo"
- **Impact**: Feature completely disappeared during architectural refactoring
- **Root Cause**: Lack of comprehensive tests and documentation

#### Recovery Process (January 2025)
- **Discovery Method**: Git archaeology using `git log -S "!"` to find original implementation
- **Recovery Date**: January 2025
- **Documentation Source**: Used existing `docs/sudo-trigger-feature.md` as implementation guide

#### Implementation Steps
1. **Code Analysis**: Found `split_respecting_quotes()` function already had `!` parsing logic
2. **Missing Logic**: Added sudo trigger detection to `replace_with_mode()` function:
   ```rust
   // Check for sudo trigger pattern: command ends with "!" (only when eol=true)
   if self.eol && !args.is_empty() {
       if let Some(last_arg) = args.last() {
           if last_arg == "!" {
               args.pop(); // Remove the "!"
               sudo = true;
               // Handle edge case: lone "!" returns empty
               if args.is_empty() {
                   return Ok(String::new());
               }
           }
       }
   }
   ```
3. **Integration**: Connected with existing sudo processing logic (wrapping, `-E` flag, etc.)
4. **Testing**: Implemented comprehensive test suite:
   - `test_sudo_trigger_comprehensive`: Basic functionality and eol requirement
   - `test_sudo_trigger_edge_cases`: Mid-command `!`, quotes, multiple `!`
   - `test_split_respecting_quotes_with_exclamation`: Parsing logic validation

#### Debugging Process
- **Initial Issue**: Manual testing with `aka --eol query "touch file !"` caused hangs
- **Root Cause Investigation**: Discovered shell interference with `!` character in double quotes
- **Solution Discovery**: Single quotes (`'...'`) prevent shell processing of `!`
- **Verification**: Created isolated test program to confirm logic worked correctly

#### Recovery Verification
- **Unit Tests**: All tests pass (`cargo test test_sudo_trigger`)
- **Manual Testing**: Works correctly with proper shell quoting
- **Integration**: Works with existing alias expansion and sudo wrapping logic

### Lessons Learned
- **Critical Importance of Tests**: Feature was lost due to lack of comprehensive tests
- **Documentation as Recovery Tool**: Existing documentation enabled accurate reconstruction
- **Shell Interaction Complexity**: Manual testing requires understanding of shell quoting
- **Regression Prevention**: Comprehensive test coverage prevents future loss

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

## Testing and Usage

### Manual Testing Commands
When testing the sudo trigger feature manually, use **single quotes** to prevent shell interference:

```bash
# ✅ CORRECT - Use single quotes
aka --eol query 'touch file !'          # → sudo touch file
aka --eol query 'ls !'                  # → sudo -E $(which eza)
aka --eol query 'systemctl restart nginx !' # → sudo systemctl restart nginx

# ❌ INCORRECT - Double quotes cause shell interference
aka --eol query "touch file !"          # → HANGS due to shell processing !
```

### Shell Interference Issue

**Problem**: The `!` character has special meaning in shells (history expansion), causing hangs when using double quotes.

**Root Cause**: In double quotes (`"..."`), shells process special characters like `!` for history expansion before passing the command to `aka`. This can cause the shell to hang while trying to expand the history.

**Solution**: Use single quotes (`'...'`) to completely protect the string from shell processing.

### Real-World Usage
In production, when `aka` is integrated into shell hooks, this quoting issue doesn't occur because the shell integration handles the command processing properly. The manual `query` command is primarily for testing and debugging.

### Test Coverage Verification
```bash
# Run the comprehensive test suite
cargo test test_sudo_trigger

# Expected output: All tests pass
# - test_sudo_trigger_comprehensive ... ok
# - test_sudo_trigger_edge_cases ... ok
```

## Future Considerations

### Potential Enhancements
1. **History integration**: Could integrate with shell history for `!!` style commands
2. **Configuration option**: Could make the trigger character configurable
3. **Multiple triggers**: Could support `!!`, `!sudo`, etc.

### Maintenance Notes
- **Critical tests**: Never remove `test_sudo_trigger_*` tests
- **Regression prevention**: Any refactoring must maintain test coverage
- **Documentation**: Keep this document updated with any changes
- **Shell testing**: Always test manual commands with single quotes to avoid shell interference

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
