use nestrs_core::activation::*;
use nestrs_core::lifetime::ServiceLifetime;
use nestrs_core::registration::*;
use nestrs_core::service::*;
use nestrs_core::{ArenaError, CompileError, ConstructionError};

fn main() {
    let _ = core::mem::size_of::<ServiceLifetime>();
}
