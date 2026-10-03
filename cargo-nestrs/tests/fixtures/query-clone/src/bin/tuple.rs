#[path = "../support.rs"]
mod support;
use support::Runner;

#[tokio::main]
async fn main() {
    let provider = support::build().await;
    {
        drop((Runner::<u64>::new(&provider),).clone());
    }
    support::finish(provider, 1, 1).await;
}
