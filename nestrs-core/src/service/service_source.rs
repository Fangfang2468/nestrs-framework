/// 编译计划携带的静态诊断来源，只用于定位执行错误。
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
