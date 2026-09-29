//! Source expressions emitted from compiler types, beyond local ADT names.

use nestrs::injectable;
use nestrs_core::{__private::REFLECTED_BINDINGS, ServiceProvider};
use std::marker::PhantomData;

#[path = "../automatic_assertions.rs"]
mod automatic_assertions;

mod generated {
    use nestrs::injectable;
    use nestrs_core::ServiceProvider;

    trait MacroPort: Send + Sync {
        fn value(&self) -> usize;
        fn identity(&self) -> usize;
    }

    // The entire type and impl come from expansion, while their containing
    // module has a physical source location. The caller supplies the name.
    macro_rules! define_service {
        ($name:ident) => {
            #[injectable]
            struct $name {
                #[value(31)]
                value: usize,
            }

            impl MacroPort for $name {
                fn value(&self) -> usize {
                    self.value
                }

                fn identity(&self) -> usize {
                    self as *const Self as usize
                }
            }
        };
    }

    define_service!(MacroService);

    pub(super) async fn assert_projection(provider: &ServiceProvider) {
        super::automatic_assertions::assert_count::<dyn MacroPort>(1);
        let concrete = nestrs_core::get_required_service!(provider, MacroService)
            .await
            .unwrap();
        let interface = nestrs_core::get_required_service!(provider, dyn MacroPort)
            .await
            .unwrap();
        assert_eq!(interface.value(), 31);
        assert_eq!(
            interface.identity(),
            concrete as *const MacroService as usize
        );
    }
}

trait RepositoryPort<T>: Send + Sync {
    fn entity_name(&self) -> &'static str;
    fn identity(&self) -> usize;
    fn label(&self) -> &str;
}

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
    #[value(String::from("text records"))]
    label: String,
}

impl<T: Send + Sync> RepositoryPort<T> for Repository<T> {
    fn entity_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn identity(&self) -> usize {
        self as *const Self as usize
    }

    fn label(&self) -> &str {
        &self.label
    }
}

// rustc internally identifies String in alloc; source emission must choose a
// legal path through the application's available extern prelude/re-exports.
type TextRepository = Repository<std::string::String>;

trait BufferPort<const CAPACITY: usize>: Send + Sync {
    fn capacity(&self) -> usize;
    fn identity(&self) -> usize;
}

#[injectable]
struct Buffer<const CAPACITY: usize> {
    #[value([3_u8; CAPACITY])]
    bytes: [u8; CAPACITY],
}

impl<const CAPACITY: usize> BufferPort<CAPACITY> for Buffer<CAPACITY> {
    fn capacity(&self) -> usize {
        self.bytes.len()
    }

    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    generated::assert_projection(&provider).await;

    let repository = nestrs_core::get_required_service!(provider, TextRepository)
        .await
        .unwrap();
    let repository_port =
        nestrs_core::get_required_service!(provider, dyn RepositoryPort<std::string::String>)
            .await
            .unwrap();
    assert_eq!(
        repository_port.entity_name(),
        std::any::type_name::<String>()
    );
    assert_eq!(repository_port.label(), "text records");
    assert_eq!(
        repository_port.identity(),
        repository as *const TextRepository as usize,
    );

    let buffer = nestrs_core::get_required_service!(provider, Buffer<8>)
        .await
        .unwrap();
    let buffer_port = nestrs_core::get_required_service!(provider, dyn BufferPort<8>)
        .await
        .unwrap();
    assert_eq!(buffer_port.capacity(), 8);
    assert_eq!(buffer_port.identity(), buffer as *const Buffer<8> as usize);
    assert_eq!(REFLECTED_BINDINGS.len(), 0);
    automatic_assertions::assert_count::<dyn RepositoryPort<std::string::String>>(1);
    automatic_assertions::assert_count::<dyn BufferPort<8>>(1);

    provider.dispose_async().await.unwrap();
    println!("auto-binding source forms: macro type/std String/const generic identities passed");
}
