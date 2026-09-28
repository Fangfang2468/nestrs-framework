# IDE acceptance fixture

The application declares only core and Tokio dependencies. The verifier copies
this standalone project below target, invokes `cargo nestrs init --vscode`, and
uses an installed, unmodified rust-analyzer LSP server with the generated model.
No editor configuration or source file in this fixture is changed by the test.
`init` prepares the existing project; the generated save-time command uses
`cargo nestrs init check`. The fixture and verifier retain their internal IDE names.

Cases cover build-script cfg/env/OUT_DIR, include-generated source, optional
features, external modules, macro-generated declarations, declaration expansion,
field/factory hover, completion, definition navigation, unsaved edits and real
errors. Compiler-side semantic discovery supplies the trait binding; ordinary
Cargo compilation is not a supported application entry point.
