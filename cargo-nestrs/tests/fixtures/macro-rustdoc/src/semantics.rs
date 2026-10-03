//! The standard rustdoc runner owns each example's code-fence semantics.
//!
//! ```compile_fail,E0308
//! let _: &str = 42_u32;
//! ```
//!
//! ```should_panic
//! panic!("the runner must observe this expected panic");
//! ```
//!
//! ```no_run
//! panic!("this example must compile without being executed");
//! ```
//!
//! ```ignore
//! compile_error!("ignored examples must stay ignored");
//! ```
//!
//! Crate-level test attributes are retained, including deny(warnings).
//!
//! ```compile_fail
//! #[deprecated]
//! fn deprecated_api() {}
//! #[warn(deprecated)]
//! fn main() { deprecated_api(); }
//! ```

use nestrs::injectable;
use nestrs_core::{ResolveError, ServiceProvider};

use crate::Configuration;

#[injectable]
pub struct Example {
    #[inject]
    configuration: Configuration,
}

impl Example {
    /// Method bodies keep real query expansions during the documentation type check.
    ///
    /// ```
    /// use nestrs_core::ServiceProvider;
    /// use nestrs_macro_rustdoc::semantics::Example;
    /// fn main() {
    ///     tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
    ///         let provider = ServiceProvider::build(None).await.unwrap();
    ///         let example = provider.get_required_service::<Example>().await.unwrap();
    ///         assert_eq!(example.configured(&provider).await.unwrap(), 5432);
    ///         provider.dispose_async().await.unwrap();
    ///         # if let Ok(path) = std::env::var("NESTRS_DOCTEST_RECORD") { std::fs::write(std::path::Path::new(&path).join("method"), "passed").unwrap(); }
    ///     });
    /// }
    /// ```
    pub async fn configured(&self, provider: &ServiceProvider) -> Result<u16, ResolveError> {
        Ok(provider.get_required_service::<Configuration>().await?.port)
    }
}

pub trait Inspect: Send + Sync {
    fn inspect(&self) -> u16;
}

impl Inspect for Example {
    /// Trait implementation documentation can declare a new downstream trait query.
    ///
    /// ```
    /// use nestrs_core::ServiceProvider;
    /// use nestrs_macro_rustdoc::semantics::Inspect;
    /// fn main() {
    ///     tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
    ///         let provider = ServiceProvider::build(None).await.unwrap();
    ///         let example = provider.get_required_service::<dyn Inspect>().await.unwrap();
    ///         assert_eq!(example.inspect(), 5432);
    ///         provider.dispose_async().await.unwrap();
    ///         # if let Ok(path) = std::env::var("NESTRS_DOCTEST_RECORD") { std::fs::write(std::path::Path::new(&path).join("trait-method"), "passed").unwrap(); }
    ///     });
    /// }
    /// ```
    fn inspect(&self) -> u16 {
        self.configuration.port
    }
}

impl Inspect for (u16, u16) {
    /// Complex impl self types still retain their documentation examples.
    ///
    /// ```
    /// use nestrs_macro_rustdoc::semantics::Inspect;
    /// assert_eq!((20_u16, 22_u16).inspect(), 42);
    /// # if let Ok(path) = std::env::var("NESTRS_DOCTEST_RECORD") { std::fs::write(std::path::Path::new(&path).join("complex-impl"), "passed").unwrap(); }
    /// ```
    fn inspect(&self) -> u16 {
        self.0 + self.1
    }
}

macro_rules! documented_item {
    () => {
        #[doc = include_str!("guides/semantics-guide.md")]
        pub fn included_documentation() {}
    };
}

documented_item!();

#[cfg(doc)]
/// Conditional documentation is collected from the actual cfg(doc) compilation.
///
/// ```
/// assert_eq!(6 * 7, 42);
/// assert_eq!(include_str!("semantics-value.txt"), "module");
/// # if let Ok(path) = std::env::var("NESTRS_DOCTEST_RECORD") { std::fs::write(std::path::Path::new(&path).join("doc-cfg"), "passed").unwrap(); }
/// ```
pub struct DocumentationOnly;
