/// 宏展开时捕获的静态注册来源。
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ServiceSource {
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
}

impl ServiceSource {
    pub const fn new(file: &'static str, line: u32, column: u32) -> Self {
        Self { file, line, column }
    }
}
