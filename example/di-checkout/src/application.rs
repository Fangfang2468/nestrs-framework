//! 应用生命周期与请求边界。只有这一层访问容器，用例通过字段接收协作者。
use std::num::NonZeroUsize;

use nestrs_core::{
    BuildError, DisposeError, InitializationMode, ResolveError, ServiceProvider,
    ServiceProviderOptions,
};
use thiserror::Error;

use crate::{
    checkout::CheckoutService,
    domain::{AuditEvent, CheckoutError, CheckoutRequest, Order, OrderStore},
    infrastructure::{Inventory, Repository},
    observe::event,
};

#[derive(Debug, Clone)]
pub(crate) struct RunOptions {
    pub initialization: InitializationMode,
    pub warm_up_scopes: bool,
    pub max_concurrent_activations: NonZeroUsize,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            initialization: InitializationMode::Lazy,
            warm_up_scopes: false,
            max_concurrent_activations: NonZeroUsize::new(4).unwrap(),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum ApplicationError {
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Dispose(#[from] DisposeError),
    #[error("应用操作失败：{operation}；同时关闭失败：{disposal}")]
    OperationAndDispose {
        operation: Box<Self>,
        disposal: DisposeError,
    },
}

type AppResult<T> = Result<T, ApplicationError>;

#[derive(Debug)]
pub(crate) struct CheckoutResponse {
    pub customer: String,
    pub result: Result<String, CheckoutError>,
}

#[derive(Debug)]
pub(crate) struct RunReport {
    pub responses: Vec<CheckoutResponse>,
    pub orders: Vec<Order>,
    pub remaining_stock: u32,
    pub audit_entries: usize,
}

pub(crate) async fn run(
    options: RunOptions,
    requests: Vec<CheckoutRequest>,
) -> AppResult<RunReport> {
    event(format!(
        "[启动] {:?}；构造并发上限 {}；scope 预热 {}",
        options.initialization, options.max_concurrent_activations, options.warm_up_scopes
    ));
    event("[启动] build 开始：加载编译计划并启动容器");
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: options.initialization,
        max_concurrent_activations: options.max_concurrent_activations,
    })
    .await?;
    match options.initialization {
        InitializationMode::Lazy => event("[启动] build 完成，Lazy 尚未实例化业务服务"),
        InitializationMode::Eager => event("[启动] build 完成，Singleton 及其必要依赖已预热"),
    }

    // 先保存操作结果，再等待关闭，避免 ? 让错误路径跳过显式 disposal。
    let outcome = process_orders(&provider, requests, options.warm_up_scopes).await;
    event("[关闭] 等待 root 的全部实例 cleanup");
    let close = provider.dispose_async().await;
    if close.is_ok() {
        event("[关闭] root 已完成");
    }
    finish_close(outcome, close)
}

async fn process_orders(
    provider: &ServiceProvider,
    requests: Vec<CheckoutRequest>,
    warm_up: bool,
) -> AppResult<RunReport> {
    let mut responses = Vec::with_capacity(requests.len());
    let mut requests = requests.into_iter();
    // 每批最多并发处理两笔订单，各自拥有 scope。这是业务请求并发，
    // 与 DI 构造任务的 max_concurrent_activations 无关。
    while let Some(first) = requests.next() {
        if let Some(second) = requests.next() {
            let (first, second) = tokio::join!(
                handle_checkout(provider, first, warm_up),
                handle_checkout(provider, second, warm_up),
            );
            // 两个请求都等待 scope 关闭后，才传播基础设施/容器错误。
            responses.push(first?);
            responses.push(second?);
        } else {
            responses.push(handle_checkout(provider, first, warm_up).await?);
        }
    }
    let orders = provider
        .get_required_service::<dyn OrderStore>()
        .await?
        .all();
    let inventory = provider.get_required_service::<Inventory>().await?;
    let audit = provider
        .get_required_service::<Repository<AuditEvent>>()
        .await?;
    Ok(RunReport {
        responses,
        orders,
        remaining_stock: inventory
            .remaining("KEYBOARD")
            .expect("本地目录包含 KEYBOARD"),
        audit_entries: audit.all().len(),
    })
}

pub(crate) async fn handle_checkout(
    provider: &ServiceProvider,
    request: CheckoutRequest,
    warm_up: bool,
) -> AppResult<CheckoutResponse> {
    let customer = request.customer.clone();
    let scope = provider.create_scope();
    event(format!("[请求 {customer}] 创建 scope"));
    let outcome = async {
        if warm_up {
            scope.warm_up().await?;
        }
        let checkout = scope
            .service_provider()
            .get_required_service::<CheckoutService>()
            .await?;
        // 编译器从查询方法识别闭合仓库并纳入计划。审计随真实业务结果写入，
        // 不由演示脚本事后补写，也不需要让结账用例依赖容器。
        let audit = scope
            .service_provider()
            .get_required_service::<Repository<AuditEvent>>()
            .await?;
        let result = checkout.place_order(request).await;
        let message = match &result {
            Ok(order) => format!(
                "request #{} 客户 {customer} 下单成功：{}",
                order.request_id, order.id
            ),
            Err(error) => format!(
                "request #{} 客户 {customer} 下单拒绝：{error}",
                checkout.context_id()
            ),
        };
        let entry = AuditEvent { message };
        audit.insert(entry.clone());
        event(format!("[审计] {}", entry.message));
        // 成功分支才获取延迟注入的收据格式器。初始化错误仍由应用层统一关闭 scope
        // 后传播，业务拒绝则保留原来的 CheckoutError，不触发额外服务构造。
        let result = match result {
            Ok(order) => Ok(checkout.format_receipt(&order).await?),
            Err(error) => Err(error),
        };
        Ok(CheckoutResponse {
            customer: customer.clone(),
            result,
        })
    }
    .await;
    let close = scope.dispose_async().await;
    if close.is_ok() {
        event(format!("[关闭] 请求 {customer} 的 scope 已完成"));
    }
    finish_close(outcome, close)
}

fn finish_close<T>(outcome: AppResult<T>, close: Result<(), DisposeError>) -> AppResult<T> {
    match (outcome, close) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(error), Err(disposal)) => Err(ApplicationError::OperationAndDispose {
            operation: Box::new(error),
            disposal,
        }),
    }
}
