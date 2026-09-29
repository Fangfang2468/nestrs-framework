//! Match exact requested interfaces without counting unrelated latent capabilities.

use nestrs_core::__private::{Injectable, REFLECTED_AUTOMATIC_BINDINGS, ServiceType};

pub fn assert_count<I: Injectable + ?Sized>(expected: usize) {
    let interface = ServiceType::create::<I>();
    let actual = REFLECTED_AUTOMATIC_BINDINGS
        .iter()
        .map(|declare| declare())
        .filter(|binding| binding.trait_type == interface)
        .count();
    assert_eq!(
        actual, expected,
        "automatic projection capability count for {}",
        interface.name,
    );
}
