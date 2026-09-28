//! This crate has no direct dependency on core or any declaration bridge.
//!
//! ```
//! let _: Option<nestrs_bridge_consumer::Exported> = None;
//! assert_eq!(nestrs_bridge_consumer::answer(), 42);
//! ```

pub use producer::Exported;

pub fn answer() -> u32 {
    producer::answer()
}
