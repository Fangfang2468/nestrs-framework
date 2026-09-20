use nestrs_core::lifetime::ServiceLifetime;
use nestrs_core::registration::*;
use nestrs_core::{ActivationError, ArenaError, CompileError};

fn main() {
    let _ = core::mem::size_of::<ServiceLifetime>();
}
