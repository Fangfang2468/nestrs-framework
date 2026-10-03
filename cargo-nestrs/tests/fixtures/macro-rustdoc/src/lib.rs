#![doc(test(attr(deny(warnings)), no_crate_inject))]
//! These examples execute through standard rustdoc and the Nestrs compiler driver.
//!
//! Library declarations remain available to a documentation test:
//!
//! ```
//! use nestrs_core::ServiceProvider;
//! use nestrs_macro_rustdoc::{Configuration, Connection};
//! fn main() {
//!     tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
//!         let provider = ServiceProvider::build().await.unwrap();
//!         assert_eq!(provider.get_required_service::<Configuration>().await.unwrap().port, 5432);
//!         assert_eq!(provider.get_required_service::<Connection>().await.unwrap().0, 5432);
//!         provider.dispose_async().await.unwrap();
//!         println!("library class and async factory example completed");
//!         # if let Ok(path) = std::env::var("NESTRS_DOCTEST_RECORD") { std::fs::write(std::path::Path::new(&path).join("library"), "passed").unwrap(); }
//!     });
//! }
//! ```
//!
//! Declarations written directly in a snippet also expand and execute:
//!
//! ```
//! use nestrs_core::ServiceProvider;
//! use nestrs::{factory, injectable, primary};
//! #[primary]
//! #[injectable]
//! struct Settings { #[value(81)] port: u16 }
//! struct Server(u16);
//! trait Endpoint: Send + Sync { fn port(&self) -> u16; }
//! impl Endpoint for Server { fn port(&self) -> u16 { self.0 } }
//! #[factory]
//! async fn server(settings: Settings) -> Server {
//!     tokio::task::yield_now().await;
//!     Server(settings.port)
//! }
//! fn main() {
//!     tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
//!         let provider = ServiceProvider::build().await.unwrap();
//!         assert_eq!(provider.get_required_service::<Server>().await.unwrap().0, 81);
//!         assert_eq!(provider.get_required_service::<dyn Endpoint>().await.unwrap().port(), 81);
//!         provider.dispose_async().await.unwrap();
//!         println!("snippet declaration and borrowed factory example completed");
//!         # if let Ok(path) = std::env::var("NESTRS_DOCTEST_RECORD") { std::fs::write(std::path::Path::new(&path).join("declarations"), "passed").unwrap(); }
//!     });
//! }
//! ```
//!
//! The compiler collects closed generic roots from ordinary queries in documentation tests:
//!
//! ```
//! use nestrs_core::ServiceProvider;
//! use nestrs::{injectable, primary};
//! #[injectable]
//! #[primary]
//! struct Repository<T> { marker: std::marker::PhantomData<T>, #[value(7)] count: usize }
//! struct User;
//! fn main() {
//!     tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
//!         let provider = ServiceProvider::build().await.unwrap();
//!         assert_eq!(provider.get_required_service::<Repository<User>>().await.unwrap().count, 7);
//!         provider.dispose_async().await.unwrap();
//!         println!("snippet closed generic example completed");
//!         # if let Ok(path) = std::env::var("NESTRS_DOCTEST_RECORD") { std::fs::write(std::path::Path::new(&path).join("generics"), "passed").unwrap(); }
//!     });
//! }
//! ```

use nestrs::{factory, injectable};

pub mod semantics;

#[injectable]
pub struct Configuration {
    #[value(5432)]
    pub port: u16,
}

pub struct Connection(pub u16);

#[factory]
async fn connection(configuration: Configuration) -> Connection {
    tokio::task::yield_now().await;
    Connection(configuration.port)
}

#[cfg(doc)]
#[injectable]
pub struct DocumentationOnly;
