//! Supported query API: each macro contributes static type information before forwarding its
//! runtime operation. The static declaration never evaluates the provider or key expression.

/// 获取默认 key 的必选服务。类型信息在编译和链接时进入静态图声明。
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

/// Shared expansion implementation; not part of the supported application API.
#[doc(hidden)]
#[macro_export]
macro_rules! __nestrs_query {
    ($operation:ident, $provider:expr, $service:ty, $key:expr) => {{
        #[$crate::__private::linkme::distributed_slice($crate::__private::REFLECTED_ROOTS)]
        #[linkme(crate = $crate::__private::linkme)]
        // The extra receiver reference deliberately selects the bounded implementation first.
        #[allow(clippy::needless_borrow)]
        fn __nestrs_query_root() -> $crate::__private::RootDeclaration {
            $crate::__private::compiler_request::<$service>();
            use $crate::__private::ProbeProvider as _;
            // Keep this lookup at the concrete expansion site. A generic helper would choose
            // its fallback during generic type checking and would not re-specialize later.
            let __nestrs_probe = $crate::__private::Probe::<$service>::new();
            $crate::__private::RootDeclaration {
                service_type: $crate::__private::ServiceType::create::<$service>(),
                materialize: (&&__nestrs_probe).provider_callback(),
                source: $crate::__private::ServiceSource::new(file!(), line!(), column!()),
            }
        }
        use $crate::__private::QueryTarget as _;
        // Resolve the view before constructing the future: its lifetime comes from the owner,
        // not the temporary receiver reference used to normalize this expression.
        let __nestrs_view = (&($provider)).__nestrs_query_view();
        $crate::__private::$operation::<$service>(__nestrs_view, $key)
    }};
}
