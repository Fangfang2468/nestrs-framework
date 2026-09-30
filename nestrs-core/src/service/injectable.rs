/// 容器可安全交给 Tokio worker 并由强 lease 保存的类型约束。
///
/// `'static` 限制类型不能包含短于容器的外部借用，并不表示实例永不释放；实例仍在
/// owner 关闭及最后一个 lease 释放后析构。此 trait 不代表服务已经注册。
pub trait Injectable: Send + Sync + 'static {}

/// 自动赋予全部线程安全、静态存活的类型可注入能力。
///
/// 因此下游 crate 不得再为自己的类型手写 `impl Injectable`；该实现会与此 blanket
/// 实现冲突。
///
/// # 类型参数
///
/// - `T`：满足 `Send + Sync + 'static` 的 Rust 类型（可为 trait object）。
impl<T> Injectable for T where T: ?Sized + Send + Sync + 'static {}
