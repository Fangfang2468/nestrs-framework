use nestrs::injectable;
#[path = "../shared.rs"]
mod shared;

#[injectable]
struct BetaOnly;
struct Beta;

fn main() {
    shared::query_only::<Beta>();
    let provider = None::<nestrs_core::ServiceProvider>;
    if let Some(provider) = provider.as_ref() {
        drop(nestrs_core::get_required_service!(
            provider,
            shared::Cache<Beta>
        ));
    }
    shared::forbidden("business main beta");
}
