//! 未被查询的服务也必须在最终入口编译时报告跨 crate 歧义。

use contracts::AmbiguousPort;
use fallback_provider as _;
use nestrs::injectable;
use primary_provider as _;

#[injectable]
struct NeedsConflict {
    #[inject("conflict")]
    _port: dyn AmbiguousPort,
}

// 这里故意不调用 ServiceProvider::build：图结构检查不依赖用户执行容器入口。
fn main() {
    if let Some(path) = std::env::var_os("NESTRS_CROSS_AMBIGUITY_SENTINEL") {
        std::fs::write(path, "invalid application main executed").unwrap();
    }
    panic!("invalid graph must be rejected during compilation");
}
