# Proc-macro rustdoc fixture

This standalone application workspace has no Nestrs macro package dependency.
The toolchain supplies its private proc-macro bridge as the `nestrs` extern crate
to the application compiler. The driver type-checks the actual documentation
crate, extracts a documentation-only HIR carrier for real rustdoc, and compiles
each example through the complete compiler pipeline. The feature-enabled `rustdoc`
integration test runs both `cargo nestrs test` and `cargo nestrs test --doc`, requires
12 passing examples and one intentionally ignored example, and verifies eight
files written after runtime assertions.
Ordinary Cargo does not inject this framework namespace.

The examples cover library declarations, declarations inside documentation,
async borrowed factory inputs, both primary orders, closed generic query roots,
resolution and disposal. New trait implementations inside a snippet and downstream
trait queries are both tested. Additional cases preserve inherent/trait method
documentation, complex tuple impl self types, macro-generated included Markdown,
cfg(doc), hidden lines, compile_fail, should_panic, no_run, ignore, and crate test
attributes including deny(warnings) and no_crate_inject. The ignored example
contains compile_error and must never be compiled.

Relative include_str/include_bytes paths are tested against distinct Rust-module
and Markdown directories, including a parent-directory path and nested include
files. A Rust source included from Markdown declares a service and trait; its
query must pass full automatic binding and real container resolution.
