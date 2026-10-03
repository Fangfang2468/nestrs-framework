/// 编译计划携带的静态诊断来源，只用于定位执行错误。
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ServiceSource {
    /// 服务声明的源文件路径。
    pub file: &'static str,

    /// 服务声明在源文件中的一基行号。
    pub line: u32,

    /// 服务声明在该行中的一基列号。
    pub column: u32,
}

impl ServiceSource {
    /// 保存生成适配器提供的声明文件、行号和列号。
    pub const fn new(file: &'static str, line: u32, column: u32) -> Self {
        Self { file, line, column }
    }
}
