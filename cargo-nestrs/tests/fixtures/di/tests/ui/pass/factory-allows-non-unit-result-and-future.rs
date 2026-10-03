use nestrs::{factory};

#[factory(key = "sync")]
fn result_factory() -> Result<u8, &'static str> {
    Ok(1)
}

#[factory(key = "async")]
async fn async_result_factory() -> Result<u8, &'static str> {
    Ok(1)
}

#[factory(key = "future")]
fn future_factory() -> impl ::core::future::Future<Output = u8> {
    async { 1 }
}

fn main() {}
