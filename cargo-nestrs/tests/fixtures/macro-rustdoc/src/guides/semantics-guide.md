This code block comes from `include_str!` inside a macro-generated item. The crate's
`no_crate_inject` setting lets it declare a local module with the library's name.
The included service is compiled through the same generated reflection and
automatic binding path as an application. This example contributes its own
validated `__nestrs_reflect_v1` entry; the ordinary query method below requires
no registration call or runtime metadata file.

```rust
mod nestrs_macro_rustdoc {
    pub const ANSWER: usize = 42;
}

include!("included/service.rs");

fn main() {
    assert_eq!(nestrs_macro_rustdoc::ANSWER, 42);
    assert_eq!(include_str!("semantics-value.txt"), "markdown");
    assert_eq!(include_bytes!("semantics-value.txt"), b"markdown");
    assert_eq!(include_str!("../semantics-value.txt"), "module");
    assert_eq!(include!("included/nested.rs"), "nested");
    tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
        let provider = nestrs_core::ServiceProvider::build().await.unwrap();
        let service = provider.get_required_service::<dyn IncludedContract>().await.unwrap();
        assert_eq!(service.answer(), 42);
        provider.dispose_async().await.unwrap();
    });
    # if let Ok(path) = std::env::var("NESTRS_DOCTEST_RECORD") { std::fs::write(std::path::Path::new(&path).join("included"), "passed").unwrap(); }
}
```
