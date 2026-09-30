//! This crate has no direct dependency on core or any declaration bridge.
//!
//! ```
//! let _: Option<nestrs_bridge_consumer::Exported> = None;
//! assert_eq!(nestrs_bridge_consumer::answer(), 42);
//! assert_eq!(nestrs_bridge_consumer::resolve_number(), 42);
//! ```

pub use producer::Exported;
pub use producer::resolve_number;

pub fn answer() -> u32 {
    producer::answer()
}
