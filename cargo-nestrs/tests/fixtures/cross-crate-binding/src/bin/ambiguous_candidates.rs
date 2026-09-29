//! This executable is intentionally invalid; other binaries never request this trait.

use contracts::AmbiguousPort;
use nestrs::injectable;
use nestrs_core::ServiceProvider;

#[injectable]
struct NeedsConflict {
    #[inject("conflict")]
    _port: dyn AmbiguousPort,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_eq!(primary_provider::total_constructions(), 0);
    assert_eq!(fallback_provider::conflict_constructions(), 0);
    let panic = tokio::spawn(ServiceProvider::build())
        .await
        .err()
        .expect("same-key candidates from different crates must be ambiguous");
    assert!(panic.is_panic());
    let payload = panic.into_panic();
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("graph failure must contain a diagnostic");
    assert!(message.contains("trait 候选不唯一"), "{message}");
    assert!(message.contains("AmbiguousPort"), "{message}");
    assert_eq!(primary_provider::total_constructions(), 0);
    assert_eq!(fallback_provider::conflict_constructions(), 0);
    eprintln!("cross-crate ambiguity: rejected before any provider construction");
    std::process::exit(23);
}
