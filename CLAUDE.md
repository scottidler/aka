# aka

High-performance shell alias manager with a file-watching daemon for instant alias reloads.

## Quick Reference

```bash
otto ci                # full CI pipeline
cargo test             # run tests
cargo build            # build both binaries
```

## Architecture

Two binaries from one crate:
- `aka` - CLI for querying and managing aliases
- `aka-daemon` - file-watching daemon that reloads aliases on config change

Config: `~/.config/aka/aka.yml`

## Install (for /shipit)

```bash
cargo install --path . && systemctl --user restart aka-daemon
```

aka-daemon runs as a systemd user service and must be restarted after install.
