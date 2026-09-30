//! Exact compiler projection assertions without a public runtime ABI.
#[path = "../../../../../cargo-nestrs/tests/support/compiler_bindings.rs"]
mod compiler_bindings;
pub use compiler_bindings::{assert_count, explicit_count};
