use nestrs_macro::factory;

#[factory]
fn result_factory() -> Result<u8, &'static str> {
    Ok(1)
}

#[factory]
async fn async_result_factory() -> Result<u8, &'static str> {
    Ok(1)
}

#[factory]
fn future_factory() -> impl ::core::future::Future<Output = u8> {
    async { 1 }
}

fn main() {}
