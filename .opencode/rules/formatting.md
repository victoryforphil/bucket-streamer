# Code Formatting

## Rust
- Use `cargo fmt` for all Rust code
- Run `cargo clippy` to catch warnings
- Format after changes, verify before commit

## Session Start
Ask user about:
- Auto-format code changes?
- Run clippy checks?

## Commands
```bash
cargo fmt              # Format code
cargo fmt --check      # Verify formatting
cargo clippy           # Check warnings
cargo clippy --fix     # Auto-fix warnings
```
