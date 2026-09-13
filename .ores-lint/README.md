# `.ores-lint`

Lint configuration for this repository, in one place.

| file | tool | how CI uses it |
|---|---|---|
| `rustfmt.toml` | rustfmt | `cargo fmt --all -- --check` |
| `clippy.toml` | clippy | `cargo clippy --locked --all-targets -- -D warnings` |

`rustfmt.toml` at the repository root is what rustfmt actually reads; the copy
here is the authority the lane keeps in sync across the org's Rust repositories.
Contracts (`tsp compile`, ajv) are linted in `gha-indie-worker-interfaces`, not
here — this repository imports the generated types rather than authoring them.
