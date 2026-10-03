#[path = "../support.rs"]
mod support;
use support::Runner;

#[tokio::main]
async fn main() {
    let provider = support::build().await;
    {
        if false {
            drop((Runner::<u64>::new(&provider),).clone());
        }
    }
    support::finish(provider, 1, 0).await;
}
