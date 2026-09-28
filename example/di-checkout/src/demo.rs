//! 应用入口层：这里决定 scope 边界并查询服务，业务服务不持有 ServiceProvider。
use std::{error::Error, io, num::NonZeroUsize};

use nestrs_core::{
    DisposeError, InitializationMode, ServiceKey, ServiceProvider, ServiceProviderOptions,
    get_keyed_service, get_required_keyed_service, get_required_service, get_service,
};

use crate::{
    domain::{AuditEvent, CheckoutError, CheckoutRequest, Order, PaymentMethod},
    observe::event,
    services::{
        CheckoutService, FraudCheck, Inventory, OrderStore, PaymentGateway, ReceiptFormatter,
        Repository, RequestContext,
    },
};

type DemoResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Clone)]
pub struct DemoOptions {
    pub initialization: InitializationMode,
    pub warm_up_scopes: bool,
    pub max_concurrent_activations: NonZeroUsize,
}

impl Default for DemoOptions {
    fn default() -> Self {
        Self {
            initialization: InitializationMode::Lazy,
            warm_up_scopes: false,
            max_concurrent_activations: NonZeroUsize::new(4).unwrap(),
        }
    }
}

#[derive(Debug)]
pub struct DemoReport {
    pub orders: Vec<Order>,
    pub remaining_stock: u32,
    pub audit_entries: usize,
}

/// 完整生命周期：即使查询或业务执行失败，也先等待已创建的 root 关闭。
pub async fn run(options: DemoOptions) -> DemoResult<DemoReport> {
    event(format!(
        "[启动] {:?}；构造并发上限 {}；scope 预热 {}",
        options.initialization, options.max_concurrent_activations, options.warm_up_scopes
    ));
    event("[启动] build 开始：验证全部静态声明并冻结依赖图");
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: options.initialization,
        max_concurrent_activations: options.max_concurrent_activations,
    })
    .await?;
    match options.initialization {
        InitializationMode::Lazy => event("[启动] build 完成，Lazy 尚未实例化业务服务"),
        InitializationMode::Eager => event("[启动] build 完成，Singleton 及其必要依赖已预热"),
    }
    let outcome = checkout_flow(&provider, &options).await;
    event("[关闭] 等待 root 的全部实例 cleanup");
    let close = provider.dispose_async().await;
    if close.is_ok() {
        event("[关闭] root 已完成");
    }
    finish_close(outcome, close)
}

async fn checkout_flow(
    provider: &ServiceProvider,
    options: &DemoOptions,
) -> DemoResult<DemoReport> {
    event("[业务] 商品 KEYBOARD：库存 5 件，单价 199.00 元");
    event("[业务] 并发受理 Alice（银行卡，1 件）和 Bob（钱包，2 件）");
    let (alice, bob) = tokio::join!(
        request_scope(
            provider,
            "A",
            request("Alice", 1, PaymentMethod::Card, "approved"),
            options.warm_up_scopes,
        ),
        request_scope(
            provider,
            "B",
            request("Bob", 2, PaymentMethod::Wallet, "approved"),
            options.warm_up_scopes,
        ),
    );
    // join 等待两条请求完成关闭后再传播错误，避免遗漏另一条请求的关闭结果。
    let alice = alice?;
    let bob = bob?;
    ensure(
        alice.context_id != bob.context_id,
        "不同请求必须具有不同的 Scoped context",
    )?;
    ensure(
        alice.store_id == bob.store_id,
        "两个请求必须共享同一订单仓库",
    )?;
    ensure(
        alice.database_id == bob.database_id,
        "两个请求必须共享同一数据库连接",
    )?;
    ensure(
        alice.formatter_id != bob.formatter_id,
        "两个消费者必须获得独立 Transient",
    )?;
    let first_order = alice.result?;
    let second_order = bob.result?;
    event(format!(
        "[验证] Scoped 隔离：context #{} / #{}；Singleton 共享：仓库 #{}、数据库 #{}",
        alice.context_id, bob.context_id, alice.store_id, alice.database_id
    ));

    event("[业务] Carol 购买 1 件，但模拟支付渠道拒绝这笔交易");
    let declined = request_scope(
        provider,
        "C",
        request("Carol", 1, PaymentMethod::Card, "declined"),
        options.warm_up_scopes,
    )
    .await?;
    ensure(
        matches!(declined.result, Err(CheckoutError::PaymentDeclined)),
        "拒绝的支付必须返回 PaymentDeclined",
    )?;
    let inventory = get_required_service!(provider, Inventory).await?;
    ensure(
        inventory.remaining("KEYBOARD") == Some(2),
        "支付失败必须归还预占库存",
    )?;
    event("[验证] 支付拒绝后库存仍为 2 件，没有留下已支付订单");

    event("[业务] Dave 购买 99 件，库存不足，应在支付前拒绝");
    let insufficient = request_scope(
        provider,
        "D",
        request("Dave", 99, PaymentMethod::Wallet, "approved"),
        options.warm_up_scopes,
    )
    .await?;
    ensure(
        matches!(insufficient.result, Err(CheckoutError::OutOfStock { .. })),
        "超库存下单必须返回 OutOfStock",
    )?;

    let first_formatter = get_required_service!(provider, ReceiptFormatter).await?;
    let second_formatter = get_required_service!(provider, ReceiptFormatter).await?;
    ensure(
        first_formatter.id() != second_formatter.id(),
        "每次直接查询 Transient 必须创建新实例",
    )?;
    event(format!(
        "[验证] 同一 root 连续查询 Transient：收据格式器 #{} / #{}",
        first_formatter.id(),
        second_formatter.id()
    ));

    let wallet = get_required_keyed_service!(
        provider,
        dyn PaymentGateway,
        ServiceKey::Named("wallet".into()),
    )
    .await?;
    event(format!(
        "[验证] keyed 查询：{} 支付渠道 #{}",
        wallet.channel(),
        wallet.id()
    ));
    ensure(
        get_keyed_service!(
            provider,
            dyn PaymentGateway,
            ServiceKey::Named("cash".into())
        )
        .await?
        .is_none(),
        "不存在的 key 必须返回 None",
    )?;
    ensure(
        get_service!(provider, dyn FraudCheck).await?.is_none(),
        "本示例没有安装可选风控插件",
    )?;
    event("[验证] cash key 与可选风控插件均未注册，返回 None");
    ensure(
        get_service!(provider, CheckoutService).await.is_err(),
        "Scoped 服务不能从 root 获取",
    )?;
    event("[验证] 从 root 查询 Scoped 服务返回生命周期错误，optional 不隐藏此错误");

    // 没有任何字段注入 Repository<AuditEvent>。查询宏使这个闭合泛型在 build 前
    // 就进入静态描述集合，这里只是按已经冻结的图获取它。
    let audit = get_required_service!(provider, Repository<AuditEvent>).await?;
    for message in [
        format!("Alice 下单成功：{}", first_order.id),
        format!("Bob 下单成功：{}", second_order.id),
        "Carol 支付拒绝，库存归还".into(),
        "Dave 库存不足，未发起支付".into(),
    ] {
        audit.insert(AuditEvent { message });
    }
    event(format!(
        "[验证] 查询宏自动纳入 Repository<AuditEvent> #{}，没有 register!",
        audit.id()
    ));
    for entry in audit.all() {
        event(format!("[审计] {}", entry.message));
    }

    // 此处只查询 trait，编译器从普通 impl 确认 Repository<Order> 并生成闭合绑定。
    let store = get_required_service!(provider, dyn OrderStore).await?;
    let orders = store.all();
    let remaining_stock = inventory.remaining("KEYBOARD").unwrap_or(0);
    ensure(orders.len() == 2, "只有两笔成功订单可以持久保存")?;
    ensure(remaining_stock == 2, "最终库存必须为 2")?;
    let total_cents: u64 = orders.iter().map(|order| order.total_cents).sum();
    event(format!(
        "[结果] 成功订单 {} 笔；剩余库存 {} 件；成交金额 {}.{:02} 元；审计 {} 条",
        orders.len(),
        remaining_stock,
        total_cents / 100,
        total_cents % 100,
        audit.all().len()
    ));
    Ok(DemoReport {
        orders,
        remaining_stock,
        audit_entries: audit.all().len(),
    })
}

fn request(customer: &str, quantity: u32, payment: PaymentMethod, token: &str) -> CheckoutRequest {
    CheckoutRequest {
        customer: customer.into(),
        sku: "KEYBOARD".into(),
        quantity,
        payment,
        payment_token: token.into(),
    }
}

struct RequestOutcome {
    result: Result<Order, CheckoutError>,
    context_id: usize,
    store_id: usize,
    database_id: usize,
    formatter_id: usize,
}

async fn request_scope(
    provider: &ServiceProvider,
    label: &str,
    request: CheckoutRequest,
    warm_up: bool,
) -> DemoResult<RequestOutcome> {
    let scope = provider.create_scope();
    event(format!("[请求 {label}] 创建 scope，本操作不构造服务"));
    let outcome: DemoResult<RequestOutcome> = async {
        if warm_up {
            event(format!("[请求 {label}] 预热当前 scope"));
            scope.warm_up().await?;
        }
        let checkout = get_required_service!(scope.service_provider(), CheckoutService).await?;
        let same_checkout =
            get_required_service!(scope.service_provider(), CheckoutService).await?;
        ensure(
            std::ptr::eq(checkout, same_checkout),
            "同一 scope 必须复用 CheckoutService",
        )?;
        let context = get_required_service!(scope.service_provider(), RequestContext).await?;
        ensure(
            context.id() == checkout.context_id(),
            "查询的 context 必须与注入的 context 相同",
        )?;
        ensure(!checkout.has_fraud_check(), "optional 风控字段应为 None")?;
        event(format!(
            "[请求 {label}] context #{}；订单仓库 #{}；数据库 #{}；收据格式器 #{}",
            checkout.context_id(),
            checkout.store_id(),
            checkout.database_id(),
            checkout.formatter_id()
        ));
        let result = checkout.place_order(request).await;
        match &result {
            Ok(order) => event(format!("[请求 {label}] {}", checkout.format_receipt(order))),
            Err(error) => event(format!("[请求 {label}] 业务拒绝：{error}")),
        }
        Ok(RequestOutcome {
            result,
            context_id: checkout.context_id(),
            store_id: checkout.store_id(),
            database_id: checkout.database_id(),
            formatter_id: checkout.formatter_id(),
        })
    }
    .await;
    // 查询得到的引用在上方块中结束借用，才能消费 scope 并等待其关闭。
    let close = scope.dispose_async().await;
    if close.is_ok() {
        event(format!("[关闭] 请求 {label} 的 scope 已完成"));
    }
    finish_close(outcome, close)
}

fn ensure(condition: bool, message: &'static str) -> DemoResult<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message).into())
    }
}

fn finish_close<T>(outcome: DemoResult<T>, close: Result<(), DisposeError>) -> DemoResult<T> {
    match (outcome, close) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(error), Err(close)) => {
            Err(io::Error::other(format!("业务或查询失败：{error}；同时关闭失败：{close}")).into())
        }
    }
}
