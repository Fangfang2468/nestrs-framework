#![allow(dead_code)]

use nestrs::factory;
use nestrs_core::LazyInjection;

struct Missing;

struct ReportService {
    missing: LazyInjection<Missing>,
}

// 故意错误：工厂的 lazy 参数也必须在编译期找到 provider；检查不会执行此工厂。
#[factory]
fn report_service(#[lazy] missing: Missing) -> ReportService {
    ReportService { missing }
}

fn main() {}
