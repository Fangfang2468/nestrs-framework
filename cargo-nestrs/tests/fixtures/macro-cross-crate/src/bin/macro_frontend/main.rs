use nestrs::{factory, injectable, primary};
use nestrs_core::ServiceProvider;
trait Port: Send + Sync {
    fn number(&self) -> u32;
}
#[injectable]
#[derive(Debug)]
#[primary]
struct Dep {
    #[value(7)]
    number: u32,
}
impl Port for Dep {
    fn number(&self) -> u32 {
        self.number
    }
}
#[injectable]
struct Other;
impl Port for Other {
    fn number(&self) -> u32 {
        99
    }
}
macro_rules! make_consumer {
    ($ty:ty) => {
        #[injectable]
        struct Consumer {
            #[inject]
            port: $ty,
        }
    };
}
make_consumer!(dyn Port);
#[cfg(any())]
#[invalid_attribute_in_disabled_code]
struct NotCompiled;
#[cfg_attr(all(), injectable)]
struct Configured;
mod external;
struct FactoryPort(u32);
#[primary]
#[factory]
async fn make(#[inject] port: dyn Port) -> FactoryPort {
    tokio::task::yield_now().await;
    FactoryPort(port.number())
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build(None).await.unwrap();
    let consumer = provider.get_required_service::<Consumer>().await.unwrap();
    assert_eq!(consumer.port.number(), 7);
    assert_eq!(
        provider
            .get_required_service::<FactoryPort>()
            .await
            .unwrap()
            .0,
        7
    );
    assert_eq!(
        provider
            .get_required_service::<external::External>()
            .await
            .unwrap()
            .number(),
        7
    );
    provider.dispose_async().await.unwrap();
    println!("macro frontend+primary+cfg+external+macro+factory+trait binding passed");
}
