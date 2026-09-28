# Macro declaration and DI regression fixture

This standalone Cargo workspace exercises the Nestrs compiler against the real
`nestrs-core` runtime and the CLI-managed private procedural macro bridge. The
application manifest has no dependency on a Nestrs macro package.

From the repository root, using the installed matching compiler toolchain:

```sh
cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --all-targets
```

The runtime and registration tests cover the same contracts as the former macro
integration suite. Declarations import `injectable`, `factory` and `primary`
from the tool-provided `nestrs` namespace and use ordinary `#[injectable]`,
`#[factory]` and `#[primary]` attributes. Declaration helpers remain `#[inject]`
and `#[value(...)]`. The thin
macros delegate to the shared backend in `cargo-nestrs`. The driver adds the
private bridge as an extern crate; normal Rust macro expansion handles the
attributes. No public macro dependency or native compiler attribute registration
is required. Runtime examples use automatic trait binding through `cargo nestrs`;
the explicit binding ABI
regressions retain the doc-hidden `#[bind]` macro.
Graph tests inspect core's read-only JSON snapshot; HTML writing and output
failures belong to the CLI integration suite.

`runtime_invalid_lifetime` runs the isolated `invalid_lifetime` binary, which
deliberately registers Singleton `ApplicationCache` depending on Scoped
`RequestSession`. The subprocess must exit with status 1, report both service
names and the declaration source, and prove no constructor ran (`构造次数 = 0`).
This regression lives in the compiler fixture rather than the successful
`example/di-checkout` application, so it cannot make the business example's
project graph invalid.

```sh
# Run the regression, including subprocess assertions.
cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --test runtime_invalid_lifetime

# Reproduce the intentionally invalid registration directly; exit status is 1.
cargo nestrs run --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --bin invalid_lifetime
```

The UI harness preserves 15 original positive cases and 37 original negative
cases, plus three ordinary macro/helper misuse negatives and one renamed macro
import positive. Two additional negatives reject `#[inject(key = ...)]` on a
field and a factory parameter, for 58 cases in total. It inherits the Nestrs
wrapper, exact compiler and private bridge when invoked through the CLI. For example:

```sh
cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --test ui
```

It checks the complete multiset of diagnostic codes and messages, rather than accepting any compilation
failure. The former stderr snapshots remain unchanged as semantic baselines;
the standalone UI build may use different source frames. The removed `register!` API
case checks that the tool-provided `nestrs` exports no registration macro. The
original stderr remains intact; this one renamed namespace is adapted exactly
by the harness, without weakening error codes, messages or duplicate counts.
Exact equivalent diagnostic spellings (`std::fmt::Debug` and qualified core
provider names) are normalized; error codes and duplicate counts are preserved.
Full compiler output is retained under `target/nestrs-ui/diagnostics` for review.

`runtime_lowering` exercises external modules, macro-generated declarations,
caller-provided paths and expressions, cfg selection, derive ordering, generic
and alias dependencies, and real factory references held across `await`.
