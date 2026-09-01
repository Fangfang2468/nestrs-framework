use nestrs_macro::primary;

#[primary]
pub struct Service;

#[primary]
struct DefaultService;

#[primary()]
struct ExplicitDefaultService;

#[primary]
fn create() {}

#[primary]
async fn create_async() {}

fn main() {}
