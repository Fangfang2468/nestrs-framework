#[path = "../support.rs"]
mod support;
use support::Runner;

#[tokio::main]
async fn main() {
    let provider = support::build().await;
    {
        let runner = Runner::<u64>::new(&provider);
        let captured = move || drop(runner);
        drop(captured.clone());
    }
    support::finish(provider, 1, 1).await;
}
