# Proc-macro declaration and external blueprint regression fixture

This is a standalone Cargo workspace. The application depends on `nestrs-core`
and Tokio. It imports declaration macros from the tool-provided `nestrs`
namespace; the CLI supplies a private proc-macro bridge without an application
manifest dependency. Build the Nestrs toolchain with
`python3 tools/build-toolchain.py`, then run
`python3 tools/verify-macro-toolchain.py` from the repository root.

- `macro_frontend` exercises imported declaration macros and plain helper attributes,
  both primary orders, derives, cfg/cfg_attr, an external source module,
  macro_rules-generated declarations, an async borrowed trait factory, automatic
  bindings, resolution and disposal.
- `external_blueprints` first closes library generic types in a binary. It checks
  metadata-driven generic dependencies, aliases, exact default/named/indexed keys
  and concrete/trait instance identity. Both debug and release executions are
  required, since optimized dependency MIR must retain the hidden type markers.
  The library keeps definition modules private and exposes renamed public
  reexports, so downstream generation must use compiler-visible public paths.
- `inherited_pair` queries a concrete/trait pair already generated upstream,
  proving that downstream analysis reuses the existing binding without adding a
  duplicate registration. Explicit duplicate bindings remain graph errors.

The verifier also runs `cargo nestrs check --all-targets` before execution to
ensure metadata-only library artifacts retain generic blueprint MIR.

This fixture proves external *closed generic blueprint* discovery. It does not
claim arbitrary cross-crate candidate collection: an unrelated upstream provider
is still not discovered solely because a downstream crate requests its trait.
