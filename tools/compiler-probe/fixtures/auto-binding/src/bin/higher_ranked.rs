//! A closed dyn type may contain lifetimes bound inside its own `for` binder.
use nestrs::injectable;
use nestrs_core::ServiceProvider;

trait TextPort<T>: Send + Sync {
    fn measure(&self, value: T) -> usize;
    fn identity(&self) -> usize;
}

#[injectable]
struct Reader {
    #[value(9)]
    base: usize,
}

impl<'a> TextPort<&'a str> for Reader {
    fn measure(&self, value: &'a str) -> usize {
        self.base + value.len()
    }

    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}

#[injectable]
struct Consumer {
    #[inject]
    reader: dyn for<'a> TextPort<&'a str>,
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let concrete = nestrs_core::get_required_service!(provider, Reader)
        .await
        .unwrap();
    let interface = nestrs_core::get_required_service!(provider, dyn for<'a> TextPort<&'a str>)
        .await
        .unwrap();
    let consumer = nestrs_core::get_required_service!(provider, Consumer)
        .await
        .unwrap();
    let owned_text = String::from("abc");
    assert_eq!(interface.measure(&owned_text), 12);
    assert_eq!(consumer.reader.measure(&owned_text), 12);
    assert_eq!(interface.identity(), concrete as *const Reader as usize);
    assert_eq!(consumer.reader.identity(), interface.identity());
    assert_eq!(nestrs_core::__private::REFLECTED_BINDINGS.len(), 1);
    provider.dispose_async().await.unwrap();
    println!("auto-binding higher-ranked interface: query/injection/identity passed");
}
