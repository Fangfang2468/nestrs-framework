use nestrs_macro::factory;

#[factory]
fn bare_result_factory() -> Result<(), &'static str> {
    Ok(())
}

#[factory]
fn core_result_factory() -> ::core::result::Result<(()), &'static str> {
    Ok(())
}

#[factory]
async fn async_result_factory() -> Result<(), &'static str> {
    Ok(())
}

fn main() {}
