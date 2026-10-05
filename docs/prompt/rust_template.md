## Goal
[What I want to achieve and why, in 1-3 sentences]

## Scope
- Files/modules to touch: [e.g. src/api/auth.rs, src/models/user.rs]
- Do NOT touch: [e.g. migrations, public API in lib.rs]

## Constraints
- No new dependencies (or: allowed crates: ...)
- Keep the public API backward compatible
- Errors: `thiserror` in the lib, `anyhow` in the binary, no `unwrap()` outside tests
- [Async / performance / `Send + Sync` / no `unsafe` / other constraints]

## Behavior
- Input: [example]
- Expected output: [example]
- Edge cases: [empty input, timeouts, invalid data...]

## Approach
Before editing, propose a short plan and wait for my OK.
(Remove this line for small, obvious changes.)

## Verification
When done, run:
- `cargo fmt`
- `cargo clippy -- -D warnings`
- `cargo test`

Fix any failure before reporting back.

## Output
- Summarize what changed and why, file by file
- Mention any trade-offs or things you were unsure about