/// 服务实例的缓存与归属规则。
///
/// 生命周期决定的是一次构造结果由哪个 owner 保存，以及后续请求能否复用它。
/// 注入依赖的合法性在完整图冻结前检查，不能靠查询时临时切换 owner 来规避。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceLifetime {
    /// 每个 root 只初始化一次，所有 scope 共享结果；失败同样缓存到 root 关闭。
    ///
    /// 即使首次查询来自 scope，构造仍归属于 root。它的整个激活依赖闭包不得
    /// 包含 Scoped，避免长期实例捕获短期请求资源。
    Singleton,

    /// 每个 scope 分别初始化一次，成功或失败结果只在该 scope 中复用。
    /// root 查询不能提供这个上下文，因此会返回生命周期错误。
    Scoped,

    /// 每次消费分别构造，包含同一消费者中请求相同服务的不同注入槽位。
    ///
    /// 实例仍由本次构造上下文的 owner 保管并清理；“不缓存”不表示可以提前释放。
    /// 若依赖闭包包含 Scoped，则只能在 scope 中查询。
    Transient,
}
