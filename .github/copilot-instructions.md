# Rust PR Review Instructions
CI overview:
* CI will build the project and run `cargo test` and `cargo clippy`.
* Feature combinations are checked with `cargo hack`.
* Do not comment on compile errors, compiler warnings, or clippy warnings.

Pay special attention to...
* code that uses async selection APIs such as `select`, `selectN`, `select_array`, `select_slice`, or is marked with a drop safety comment. These functions drop the futures that don't finish. Check that values are not lost when this happens.
* code that could possibly panic or is marked with a panic safety comment.