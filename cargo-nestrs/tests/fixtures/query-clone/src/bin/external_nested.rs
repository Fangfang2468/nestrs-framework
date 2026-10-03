#[path = "../support.rs"]
mod support;
use support::Runner;

#[tokio::main]
async fn main() {
    let provider = support::build().await;
    {
        plain_clone_helper::nested(Runner::<u64>::new(&provider));
    }
    support::finish(provider, 1, 1).await;
}
