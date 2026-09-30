//! A closed dyn type may contain lifetimes bound inside its own `for` binder.
use nestrs::injectable;
use nestrs_core::ServiceProvider;

#[path = "../automatic_assertions.rs"]
mod automatic_assertions;

trait TextPort<T>: Send + Sync {
    fn measure(&self, value: T) -> usize;
    fn identity(&self) -> usize;
}

trait NestedPort<T>: Send + Sync {
    fn nested_identity(&self) -> usize;
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

impl<T> NestedPort<T> for Reader {
    fn nested_identity(&self) -> usize {
        self as *const Self as usize
    }
}

#[injectable]
struct Consumer {
    #[inject]
    reader: dyn for<'a> TextPort<&'a str>,
    #[inject]
    nested: dyn for<'a> NestedPort<(&'a str, for<'b> fn(&'b str))>,
    #[inject]
    capturing: dyn for<'a> NestedPort<fn(&'a str, &str)>,
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
    assert_eq!(consumer.nested.nested_identity(), interface.identity());
    assert_eq!(consumer.capturing.nested_identity(), interface.identity());
    assert_eq!(automatic_assertions::explicit_count(), 0);
    automatic_assertions::assert_count::<dyn for<'a> TextPort<&'a str>>(1);
    automatic_assertions::assert_count::<dyn for<'a> NestedPort<(&'a str, for<'b> fn(&'b str))>>(1);
    automatic_assertions::assert_count::<dyn for<'a> NestedPort<fn(&'a str, &str)>>(1);
    provider.dispose_async().await.unwrap();
    println!("auto-binding higher-ranked interface: query/injection/identity passed");
}
