#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ServiceSource {
    /// 服务定义所在文件。
    pub file: &'static str,

    /// 服务定义所在文件中的行数。
    pub line: u32,

    /// 服务定义所在文件中的列数。
    pub column: u32,
}

impl ServiceSource {
    /// 用完整的静态调用点信息构造注册来源。
    pub const fn new(
        file: &'static str,
        line: u32,
        column: u32,
    ) -> Self {
        Self {
            file,
            line,
            column,
        }
    }


    /// 从调用 `caller()` 的位置推导有限来源信息。
    #[track_caller]
    pub fn caller() -> Self {
        let location = std::panic::Location::caller();

        Self::new(
            location.file(),
            location.line(),
            location.column()
        )
    }
}
