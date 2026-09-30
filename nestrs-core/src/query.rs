//! 面向业务的查询宏：先贡献静态类型声明，再转发运行时查询。
//!
//! 描述回调只记录类型，不求值 provider 或 key 表达式；运行时部分各求值一次。
//! 编译器会汇总所有有效调用点，因此查询所在分支尚未执行时，其闭合根也已进入图。

/// 获取默认 key 的必选服务。编译器将类型信息汇总到最终入口的静态图声明。
///
/// ```ignore
/// let service = nestrs_core::get_required_service!(provider, Repository<User>).await?;
/// ```
/// `provider` 可以是 root 或 scope 的 `service_provider()` 视图；只求值一次且不会被
/// 消费。返回引用借用实际 owner。服务不存在或初始化失败时返回 `ResolveError`。
/// 服务类型须是可在当前作用域独立命名的闭合类型，不能捕获外层泛型参数或 impl 的
/// `Self`；在这些位置请使用具体类型名称或闭合类型别名。
#[macro_export]
macro_rules! get_required_service {
    ($provider:expr, $service:ty $(,)?) => {
        $crate::__nestrs_query!(
            query_required,
            $provider,
            $service,
            ::core::option::Option::None
        )
    };
}

/// 获取默认 key 的可选服务。只有服务未注册时返回 `None`，其余失败返回错误。
///
/// 与必选查询相同，宏会让可物化的闭合泛型在 build 期间进入图；宏所在分支即使不
/// 执行，也会贡献静态声明。已被 `cfg` 排除的调用点不贡献声明。
#[macro_export]
macro_rules! get_service {
    ($provider:expr, $service:ty $(,)?) => {
        $crate::__nestrs_query!(
            query_optional,
            $provider,
            $service,
            ::core::option::Option::None
        )
    };
}

/// 按 key 获取必选服务。provider 和 key 表达式仅在查询时各求值一次。
///
/// 动态 key 只选择已经冻结的查询路由，不修改 provider 定义的 key，也不扩展图。
#[macro_export]
macro_rules! get_required_keyed_service {
    ($provider:expr, $service:ty, $key:expr $(,)?) => {
        $crate::__nestrs_query!(
            query_required,
            $provider,
            $service,
            ::core::option::Option::Some($key)
        )
    };
}

/// 按 key 获取可选服务，未注册的精确类型/key 返回 `None`。
#[macro_export]
macro_rules! get_keyed_service {
    ($provider:expr, $service:ty, $key:expr $(,)?) => {
        $crate::__nestrs_query!(
            query_optional,
            $provider,
            $service,
            ::core::option::Option::Some($key)
        )
    };
}

/// 四个查询宏共用的展开实现，不属于稳定的业务查询接口。
#[doc(hidden)]
#[macro_export]
macro_rules! __nestrs_query {
    ($operation:ident, $provider:expr, $service:ty, $key:expr) => {{
        #[allow(dead_code)]
        // 双重借用用于优先选择具备 ProviderDefinition 约束的探测实现。
        #[allow(clippy::needless_borrow)]
        fn __nestrs_query_root() -> $crate::registration::root::RootDeclaration {
            $crate::registration::compiler::compiler_request::<$service>();
            use $crate::registration::root::ProbeProvider as _;
            // 必须在具体类型的展开点完成探测。若移入普通泛型 helper，Rust 会在
            // 检查泛型函数时固定选择 fallback，之后不会随闭合实参重新选择实现。
            let __nestrs_probe = $crate::registration::root::Probe::<$service>::new();
            $crate::registration::root::RootDeclaration {
                service_type: $crate::service::ServiceType::create::<$service>(),
                materialize: (&&__nestrs_probe).provider_callback(),
                source: $crate::service::ServiceSource::new(file!(), line!(), column!()),
            }
        }
        use $crate::facade::QueryTarget as _;
        // 先取得 owner 的借用视图，再创建 future；结果不能借用仅为接收者归一化
        // 而产生的临时引用，否则链式 service_provider() 查询会得到过短的借用期。
        let __nestrs_view = (&($provider)).__nestrs_query_view();
        $crate::facade::$operation::<$service>(__nestrs_view, $key)
    }};
}
