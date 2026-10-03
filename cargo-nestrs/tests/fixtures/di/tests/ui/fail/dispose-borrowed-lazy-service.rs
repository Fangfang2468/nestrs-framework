use nestrs::injectable;
use nestrs_core::ServiceProvider;

#[injectable]
struct Report {
    value: usize,
}

#[injectable]
struct Consumer {
    #[inject]
    #[lazy]
    report: Report,
}

async fn close_while_lazy_target_is_borrowed(provider: ServiceProvider) {
    let consumer = provider.get_required_service::<Consumer>().await.unwrap();
    let report = consumer.report.get().await.unwrap();
    provider.dispose_async().await.unwrap();
    println!("{}", report.value);
}

fn main() {}
