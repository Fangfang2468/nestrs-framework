//! Cargo 子命令可执行入口；参数处理和退出状态集中在 commands 模块。

/// 将进程参数交给统一命令分发器，并保留其退出状态。
fn main() -> std::process::ExitCode {
    cargo_nestrs::commands::run()
}
