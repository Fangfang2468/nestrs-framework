//! 关联常量中的函数指针不能绕过查询根的有限类型增长保护。
use nestrs_core::ServiceProvider;
use std::marker::PhantomData;

struct Holder<T>(PhantomData<T>);

impl<T: Send + Sync + 'static> Holder<T> {
    const QUERY: fn(&ServiceProvider) = growing::<T>;
}

fn growing<T: Send + Sync + 'static>(provider: &ServiceProvider) {
    drop(provider.get_service::<T>());
    if false {
        Holder::<(T,)>::QUERY(provider);
    }
}

fn never_called(provider: &ServiceProvider) {
    Holder::<u8>::QUERY(provider);
}

fn main() {}
