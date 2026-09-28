# Graph CLI integration fixture

From the repository root, run:

```sh
python3 tools/verify-graph.py
# Reuse the currently built CLI/driver:
python3 tools/verify-graph.py --skip-build
```

The application manifest depends on core without a public macro package; service
declarations import the tool-provided `nestrs` namespace. The verifier snapshots
the CLI/driver binaries and private macro bridge, then drives the real `graph` and
`run` commands. Its report, HTML and captured output live under
`target/nestrs-graph-verification`.

`alpha` and `beta` have distinct query-only closed generic roots and provider
declarations. The shared graph contains a keyed factory/trait binding, a Scoped
consumer and a missing optional trait. Main, `Default`, value expressions,
factory and cleanup write a sentinel then panic if executed. Alternating graph
targets and an ordinary run verifies cache isolation in both directions. The
ordinary run is a positive control: it must execute the business-main sentinel.

Invalid dependency graphs, a binary without a direct core dependency, raw
`#![no_main]`, and `cfg_attr`-introduced `no_main` must fail without executing
application code or replacing a preexisting HTML output in explicit `--bin`
mode. Package reports include those failures while retaining valid graphs, ignore
`default-run = "alpha"` for selection, and stay deterministic across repeated
exports. A report is also written when every selected entry fails. Workspace
reports keep duplicate `alpha` names in distinct packages separate, inventory a
library-only package without fabricating a graph, and never mix their caches.
A binary named `build-script-build` shares the build script's rustc crate name;
its graph must still use its own source entry, and the build script must not be
rewritten as a graph executable. The build script only emits a benign cfg.

`feature_app` requires `extras`; `feature-alias` forwards to it. Reports mark it
skipped until Cargo enables the feature. `compile_error` requires `broken` and
proves that a compilation failure does not stop the other entries. Its normal
entry and the gated feature's value/main have sentinel traps as well. The verifier
also checks missing-feature rejection for explicit `--bin`, no-binary selection,
default/relative output paths and a failed export. Source hashes prove the CLI did
not modify any fixture source or manifest.
The first package export uses `--color=always`; diagnostics must remain stable
against an uncolored repeat. Explicit workspace `--features` is rejected before
entry builds and must preserve an existing report; per-package feature selection
is the supported route. The fixture's default members are the root package and
`other-app`, so the same preservation check also covers feature selection without
an explicit `-p` or `--workspace` selector.

Do not run this workspace through ordinary `cargo test --all-targets`: these
binaries are deliberately invalid or contain side-effect traps. The verifier
selects one target and its expected outcome at a time.
