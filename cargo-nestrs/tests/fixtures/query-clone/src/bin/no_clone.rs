#[path = "../support.rs"]
mod support;
use support::Runner;

#[tokio::main]
async fn main() {
    let provider = support::build().await;
    {
        drop((Runner::<u64>::new(&provider),));
        let ordinary = (1u64,);
        assert_eq!(ordinary.clone(), (1,));
    }
    support::finish(provider, 0, 0).await;
}
