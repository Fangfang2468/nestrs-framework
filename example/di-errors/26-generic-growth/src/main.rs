#![allow(dead_code)]

use nestrs_core::ServiceProvider;

fn growing<T: Send + Sync + 'static>(provider: &ServiceProvider) {
    drop(provider.get_service::<T>());
    // 故意让查询类型不断增加一层元组；未执行分支仍参与编译期查询分析。
    if false {
        growing::<(T,)>(provider);
    }
}

fn closed_root(provider: &ServiceProvider) {
    growing::<u8>(provider);
}

fn main() {}
