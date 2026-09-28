//! 使用真实宏注册与业务服务，覆盖两种初始化模式下同一套下单和生命周期契约。

use nestrs_core::{
    InitializationMode, ServiceKey, ServiceProvider, ServiceProviderOptions, get_keyed_service,
    get_required_keyed_service, get_required_service, get_service,
};
use nestrs_di_example::{
    domain::{AuditEvent, CheckoutError, CheckoutRequest, Order, PaymentMethod},
    services::{
        CheckoutService, Database, FraudCheck, Inventory, OrderStore, PaymentGateway,
        ReceiptFormatter, Repository,
    },
};

fn request(customer: &str, sku: &str, quantity: u32, payment: PaymentMethod) -> CheckoutRequest {
    CheckoutRequest {
        customer: customer.into(),
        sku: sku.into(),
        quantity,
        payment,
        payment_token: "approved".into(),
    }
}

fn assert_saved_orders(store: &dyn OrderStore, expected: &[&Order]) {
    let orders = store.all();
    assert_eq!(orders.len(), expected.len());
    for expected in expected {
        let actual = orders
            .iter()
            .find(|order| order.id == expected.id)
            .expect("successful order should be persisted");
        assert_eq!(actual.request_id, expected.request_id);
        assert_eq!(actual.customer, expected.customer);
        assert_eq!(actual.sku, expected.sku);
        assert_eq!(actual.quantity, expected.quantity);
        assert_eq!(actual.total_cents, expected.total_cents);
        assert_eq!(actual.payment_reference, expected.payment_reference);
    }
}

async fn exercise_checkout(initialization: InitializationMode) {
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization,
        ..Default::default()
    })
    .await
    .expect("example graph should build");
    let first_scope = provider.create_scope();
    let second_scope = provider.create_scope();
    let (first, second) = tokio::join!(
        get_required_service!(first_scope.service_provider(), CheckoutService),
        get_required_service!(second_scope.service_provider(), CheckoutService),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    let first_again = get_required_service!(first_scope.service_provider(), CheckoutService)
        .await
        .unwrap();
    assert!(std::ptr::eq(first, first_again));
    assert_ne!(first.context_id(), second.context_id());
    assert_eq!(first.store_id(), second.store_id());
    assert_eq!(first.database_id(), second.database_id());
    assert_eq!(first.formatter_id(), first_again.formatter_id());
    assert_ne!(first.formatter_id(), second.formatter_id());
    assert!(get_service!(provider, CheckoutService).await.is_err());

    let store = get_required_service!(provider, dyn OrderStore)
        .await
        .unwrap();
    let repository = get_required_service!(provider, Repository<Order>)
        .await
        .unwrap();
    let scoped_repository =
        get_required_service!(second_scope.service_provider(), Repository<Order>)
            .await
            .unwrap();
    assert!(std::ptr::eq(repository, scoped_repository));
    assert_eq!(repository.id(), store.instance_id());
    assert_eq!(store.instance_id(), first.store_id());
    assert_eq!(store.database_id(), first.database_id());
    assert_eq!(repository.database_id(), first.database_id());
    let root_database = get_required_service!(provider, Database).await.unwrap();
    let scoped_database = get_required_service!(first_scope.service_provider(), Database)
        .await
        .unwrap();
    assert!(std::ptr::eq(root_database, scoped_database));
    assert_eq!(root_database.id(), first.database_id());

    let formatter_one = get_required_service!(first_scope.service_provider(), ReceiptFormatter)
        .await
        .unwrap();
    let formatter_two = get_required_service!(first_scope.service_provider(), ReceiptFormatter)
        .await
        .unwrap();
    assert!(!std::ptr::eq(formatter_one, formatter_two));
    assert_ne!(formatter_one.id(), formatter_two.id());
    assert_ne!(formatter_one.id(), first.formatter_id());
    assert!(!first.has_fraud_check());
    assert!(!second.has_fraud_check());
    assert!(
        get_service!(provider, dyn FraudCheck)
            .await
            .unwrap()
            .is_none()
    );

    let card = get_required_keyed_service!(
        provider,
        dyn PaymentGateway,
        ServiceKey::Named("card".into())
    )
    .await
    .unwrap();
    let wallet = get_required_keyed_service!(
        provider,
        dyn PaymentGateway,
        ServiceKey::Named("wallet".into())
    )
    .await
    .unwrap();
    assert_eq!(card.channel(), "card");
    assert_eq!(wallet.channel(), "wallet");
    assert_ne!(card.id(), wallet.id());
    assert!(
        get_service!(provider, dyn PaymentGateway)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        get_keyed_service!(
            provider,
            dyn PaymentGateway,
            ServiceKey::Named("unregistered".into())
        )
        .await
        .unwrap()
        .is_none()
    );

    let inventory = get_required_service!(provider, Inventory).await.unwrap();
    assert_eq!(inventory.remaining("KEYBOARD"), Some(5));
    assert!(store.all().is_empty());
    let (first_order, second_order) = tokio::join!(
        first.place_order(request("Alice", "KEYBOARD", 1, PaymentMethod::Card)),
        second.place_order(request("Bob", "KEYBOARD", 2, PaymentMethod::Wallet)),
    );
    let first_order = first_order.unwrap();
    let second_order = second_order.unwrap();
    assert_ne!(first_order.id, second_order.id);
    assert_eq!(first_order.request_id, first.context_id());
    assert_eq!(second_order.request_id, second.context_id());
    assert_eq!(first_order.customer, "Alice");
    assert_eq!(second_order.customer, "Bob");
    assert_eq!(first_order.sku, "KEYBOARD");
    assert_eq!(second_order.sku, "KEYBOARD");
    assert_eq!(first_order.quantity, 1);
    assert_eq!(second_order.quantity, 2);
    assert_eq!(first_order.total_cents, 19_900);
    assert_eq!(second_order.total_cents, 39_800);
    assert!(!first_order.payment_reference.is_empty());
    assert!(!second_order.payment_reference.is_empty());
    assert!(first_order.payment_reference.starts_with("card-"));
    assert!(second_order.payment_reference.starts_with("wallet-"));
    let receipt = first.format_receipt(&first_order);
    assert!(receipt.contains(&first_order.id));
    assert!(receipt.contains("Alice"));
    assert!(receipt.contains("199.00"));
    assert_saved_orders(store, &[&first_order, &second_order]);
    assert_eq!(inventory.remaining("KEYBOARD"), Some(2));

    let mut declined = request("Charlie", "KEYBOARD", 1, PaymentMethod::Card);
    declined.payment_token = "declined".into();
    assert!(matches!(
        first.place_order(declined).await,
        Err(CheckoutError::PaymentDeclined)
    ));
    assert_eq!(inventory.remaining("KEYBOARD"), Some(2));
    assert_saved_orders(store, &[&first_order, &second_order]);

    assert!(matches!(
        first.place_order(request("Charlie", "KEYBOARD", 99, PaymentMethod::Card)).await,
        Err(CheckoutError::OutOfStock { sku, requested: 99, available: 2 }) if sku == "KEYBOARD"
    ));
    assert!(matches!(
        first
            .place_order(request("Charlie", "KEYBOARD", 0, PaymentMethod::Card))
            .await,
        Err(CheckoutError::InvalidQuantity)
    ));
    assert!(matches!(
        first.place_order(request("Charlie", "UNKNOWN", 1, PaymentMethod::Card)).await,
        Err(CheckoutError::UnknownProduct(sku)) if sku == "UNKNOWN"
    ));
    assert_eq!(inventory.remaining("KEYBOARD"), Some(2));
    assert_eq!(inventory.remaining("UNKNOWN"), None);
    assert_saved_orders(store, &[&first_order, &second_order]);

    // 查询发现另一个闭合泛型：它具有独立的表，仍共享同一个 Database。
    let audit = get_required_service!(provider, Repository<AuditEvent>)
        .await
        .unwrap();
    assert_ne!(audit.id(), repository.id());
    assert_eq!(audit.database_id(), repository.database_id());
    assert!(audit.all().is_empty());
    audit.insert(AuditEvent {
        message: "checkout verified".into(),
    });
    assert_eq!(audit.all()[0].message, "checkout verified");
    assert_eq!(repository.all().len(), 2);

    first_scope.dispose_async().await.unwrap();
    second_scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
}

#[tokio::test]
async fn lazy_checkout_preserves_business_and_di_contracts() {
    exercise_checkout(InitializationMode::Lazy).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eager_checkout_preserves_business_and_di_contracts() {
    exercise_checkout(InitializationMode::Eager).await;
}
