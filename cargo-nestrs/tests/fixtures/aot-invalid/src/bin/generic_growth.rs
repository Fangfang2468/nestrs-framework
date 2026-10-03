//! 编译器不能因为未执行的泛型递归陷入无限查询根物化。
use nestrs_core::ServiceProvider;

fn growing<T: Send + Sync + 'static>(provider: &ServiceProvider) {
    drop(provider.get_service::<T>());
    if false {
        growing::<(T,)>(provider);
    }
}

fn never_called(provider: &ServiceProvider) {
    growing::<u8>(provider);
}

fn main() {}
