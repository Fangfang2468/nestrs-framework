//! Inspect the matching compiler report without exposing runtime registration internals.
//!
//! Reports are isolated by compiler and tool fingerprint. A fixture can have both
//! metadata-only and linked builds; all successful reports for its crate must
//! agree on projection pairs before any assertion uses them.
#![allow(dead_code)]

use serde_json::Value;
use std::{fs, path::PathBuf, sync::OnceLock};

fn report() -> &'static Value {
    static REPORT: OnceLock<Value> = OnceLock::new();
    REPORT.get_or_init(|| {
        let mut pending = vec![PathBuf::from(env!("NESTRS_COMPILER_OUTPUT"))];
        let mut selected: Option<Value> = None;
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                } else if entry.file_name() == "analysis.json" {
                    let value: Value =
                        serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
                    if value["crate"] != env!("CARGO_CRATE_NAME") {
                        continue;
                    }
                    let status_path = entry.path().with_file_name("compilation.json");
                    let Ok(status) = fs::read(status_path) else {
                        continue;
                    };
                    let status: Value = serde_json::from_slice(&status).unwrap();
                    if status["passed"] != true {
                        continue;
                    }
                    if let Some(previous) = &selected {
                        assert_eq!(
                            previous["automatic_projections"], value["automatic_projections"],
                            "compiler passes/profiles disagree on automatic pairs"
                        );
                        assert_eq!(
                            previous["explicit_projections"], value["explicit_projections"],
                            "compiler passes/profiles disagree on explicit pairs"
                        );
                    }
                    selected = Some(value);
                }
            }
        }
        selected.expect("fixture must have a successful compiler analysis report")
    })
}

pub fn automatic_pairs() -> impl Iterator<Item = (&'static str, &'static str)> {
    report()["automatic_projections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| (pair[0].as_str().unwrap(), pair[1].as_str().unwrap()))
}

pub fn explicit_count() -> usize {
    report()["explicit_projections"].as_array().unwrap().len()
}

pub fn count<I: ?Sized>() -> usize {
    automatic_pairs()
        .filter(|(_, interface)| *interface == std::any::type_name::<I>())
        .count()
}

pub fn pair_count<C, I: ?Sized>() -> usize {
    automatic_pairs()
        .filter(|(concrete, interface)| {
            *concrete == std::any::type_name::<C>() && *interface == std::any::type_name::<I>()
        })
        .count()
}

pub fn assert_count<I: ?Sized>(expected: usize) {
    assert_eq!(
        count::<I>(),
        expected,
        "automatic projection capability count for {}",
        std::any::type_name::<I>()
    );
}
