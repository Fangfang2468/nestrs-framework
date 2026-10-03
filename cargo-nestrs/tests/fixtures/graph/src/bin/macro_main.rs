//! 图导出只读编译计划，macro_rules 生成 main 不需要专门的入口改写协议。

#[nestrs::injectable]
struct MacroMainService;

macro_rules! application_entry {
    () => {
        fn main() {
            let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
            std::fs::write(path, "macro-generated business main").unwrap();
        }
    };
}

application_entry!();
