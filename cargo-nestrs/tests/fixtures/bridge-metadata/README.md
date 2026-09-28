# Private bridge metadata regression

Only `producer` depends on `nestrs-core` and declares a service. The consumer
library and binary have no direct core or macro dependency. They must still
decode the producer's metadata, whose compiler dependency includes the private
host proc-macro bridge. The CLI driver supplies its directory as a dependency
search path to every compilation unit and rustdoc invocation.

`cargo-nestrs/tests/bridge_metadata.rs` verifies metadata-only checking, a real
build/run, and an executable consumer doctest with no `RUSTFLAGS` workaround.
