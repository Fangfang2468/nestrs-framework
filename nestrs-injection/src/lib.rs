pub mod registration;
pub mod lifetime;




#[doc(hidden)]
pub mod __private {
    /// 仅供宏生成的分布式注册静态项使用的 `linkme` crate 重导出。
    ///
    /// 下游应用无需也不应为了 DI 注册而直接依赖此名称。
    pub use linkme;
}