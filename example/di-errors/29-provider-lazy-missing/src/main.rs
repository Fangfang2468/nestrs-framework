#![allow(dead_code)]

use nestrs::{injectable, lazy};

struct Missing;

// 服务级 lazy 只决定是否作为自主预热根，不把字段改成 LazyInjection。
#[lazy]
#[injectable]
struct DeferredApplication {
    // 故意错误：即使服务从未被查询，普通注入字段仍必须有对应 provider。
    #[inject]
    missing: Missing,
}

fn main() {}
