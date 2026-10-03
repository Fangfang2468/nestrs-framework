#[path = "../support.rs"]
mod support;
use support::Runner;

#[tokio::main]
async fn main() {
    let provider = support::build().await;
    {
        drop((Runner::<u64>::new(&provider),).clone());
        drop((Runner::<u32>::new(&provider),).clone());
    }
    support::finish(provider, 2, 2).await;
}
