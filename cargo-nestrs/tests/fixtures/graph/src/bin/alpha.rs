use nestrs::injectable;
#[path = "../shared.rs"]
mod shared;

#[injectable]
struct AlphaOnly;
struct Alpha;

fn main() {
    shared::query_only::<Alpha>();
    let provider = None::<nestrs_core::ServiceProvider>;
    // The call is never run by graph, but contributes Cache<Alpha> to the graph.
    if let Some(provider) = provider.as_ref() {
        drop(provider.get_required_service::<shared::Cache<Alpha>>());
    }
    shared::forbidden("business main alpha");
}
