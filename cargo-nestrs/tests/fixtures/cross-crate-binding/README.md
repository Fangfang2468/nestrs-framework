# Cross-crate automatic binding fixture

This standalone workspace expresses application requirements for automatic bindings
across separately compiled crates. Application packages depend on `nestrs-core`
and business crates. Declaration macros come from the tool-injected `nestrs`
bridge. No fixture source uses `bind`, and no implementation type is made public
solely for generation.

The dependency layout is:

```text
contracts (no core dependency)
  ↑                    ↑
primary-provider   fallback-provider
  ↑                    │
upstream-consumer       │
  ↑                    │
final binaries ← sibling-consumer → contracts
```

`sibling-consumer` has no manifest dependency on either provider. The upstream
consumer also requires a tracking service supplied only by the fallback sibling,
which is absent from its own dependency list. `contracts` reexports interfaces
from a private module under their public business names.
`primary-provider` reexports its catalog class from a private module and keeps
its async factory return type entirely private. The two provider crates both
declare a concrete type named `Service`; they remain distinct Rust identities.

Only the final binaries contain DI query macros. Provider libraries have no
dummy request that would force a local automatic binding, no registration helper
called by the application, and no handwritten projection. Library consumers
declare ordinary injected fields.

| Binary | Runtime assertions |
| --- | --- |
| `downstream_demand` | An upstream class and private async factory satisfy a later consumer's requests; an external blanket impl supplies a second interface for the private factory type, all projections share Singleton identity, two scopes share the private Singleton, cleanup and Drop run once |
| `lazy_cross_crate` | A lazy field declared in a library selects an upstream private async factory through a trait; constructing the consumer does not construct its target, and later accesses share the Singleton with root queries |
| `sibling_selection` | A contracts-only consumer receives providers from sibling crates; primary applies to default key, named audit key remains independent, optional present/absent injection follows the selected routes |
| `transitive_reuse` | Demands repeated in two consumer crates and the binary reuse the same instances and one logical route per actual concrete/interface pair, including the private factory projection |
| `alias_identity` | Renamed trait/type exports use legal public paths, same-named types from different crates remain distinct, and one projection pair serves default and named concrete providers |
| `generic_capabilities` | Downstream dyn-only queries materialize a private closed generic and its private trait dependency; associated types, generic trait arguments, supertrait methods and explicit Send/Sync shapes share one instance |
| `higher_ranked_capabilities` | A private upstream provider implements interfaces with `for<'a>` supertraits; unbound and lifetime-dependent associated types retain their binders, all valid projections share one instance, and a request for an incompatible `'static` associated type remains absent |
| `ambiguous_candidates` | Expected CLI failure: two cross-crate providers implement the requested interface at the same key without a primary; the executable proves both construction counters remain zero and exits nonzero, while graph export preserves an existing file |

All pointer-identity assertions use nonzero-sized concrete types. The inspection
of the hidden automatic-binding directory in `transitive_reuse` only proves the
provider advertised its projection. Successful graph construction and shared
instances prove logical deduplication; the physical directory may contain
repeated descriptions. This is an internal regression assertion, not an
application API recommendation.

Both providers implement an unrequested `DormantPort` without primary. The
primary crate also contains a closed impl for an unrequested generic blueprint
whose required dependency is missing. The graph must neither reject dormant
trait candidates as ambiguous nor materialize that unused generic blueprint.
The separate usable private `Repository<UserEntity>` also stays unmaterialized
in every binary except `generic_capabilities`; this is asserted in graph JSON.
`generic_capabilities` links the provider crates only through `use provider as _`
and does not call their public functions or name their concrete types.

Run the full check/debug/release/graph matrix from the repository root:

```sh
python3 tools/verify-cross-crate-binding.py
```

`--skip-build` reuses the current compiled toolchain. The verifier copies the CLI,
driver and bridge as one snapshot, writes command diagnostics and
`target/nestrs-cross-crate-binding/report.json`, and verifies that all fixture
sources, manifests and lockfiles remain unchanged. Graph assertions check the
actual provider identities and selected dependency targets across crates.

Individual commands, after `python3 tools/build-toolchain.py`:

```sh
target/debug/cargo-nestrs check --locked --all-targets \
  --manifest-path cargo-nestrs/tests/fixtures/cross-crate-binding/Cargo.toml

target/debug/cargo-nestrs run --locked \
  --manifest-path cargo-nestrs/tests/fixtures/cross-crate-binding/Cargo.toml \
  --bin downstream_demand
```

Run the other positive binaries by changing `--bin`, and repeat with `--release`
to exercise dependency metadata under optimization. `cargo nestrs graph` can
also select each of these real binary link units. Select a positive `--bin` for
a successful single graph: exporting every entry also includes the deliberately
invalid `ambiguous_candidates`, so the complete project report records that
failure and returns a nonzero exit status. A successful type check alone
does not prove cross-crate registration, projection, or graph validation; the
runtime assertion and graph commands are distinct evidence.

The verifier records the exact toolchain artifact hashes, each command outcome,
and the host's source-preservation checks. Support on another host or a changed
compiler adapter must be established by running this suite there; adding that
host to CI is not itself evidence that its run has passed.
