# Proc-macro rustdoc fixture

This standalone application workspace has no Nestrs macro package dependency.
The toolchain supplies its private proc-macro bridge as the `nestrs` extern crate
to both the application compiler and real rustdoc. The feature-enabled `rustdoc`
integration test runs both `cargo nestrs test` and `cargo nestrs test --doc`, requires three executed examples
with zero ignored tests, and checks files written after their runtime assertions.
Ordinary Cargo does not inject this framework namespace.

The examples cover library declarations, declarations inside documentation,
async borrowed factory inputs, both primary orders, closed generic query roots,
resolution and disposal. They use concrete queries; automatic trait discovery
inside a standalone documentation snippet is not part of this test's contract.
