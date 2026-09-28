//! Literal injection keys resolve through real declarations, automatic bindings and activation.
use nestrs::{factory, injectable};
use nestrs_core::{ServiceProvider, get_required_service};

trait PaymentGateway: Send + Sync {
    fn charge(&self, subtotal: u64) -> u64;
}

struct Gateway {
    fee: u64,
}

impl PaymentGateway for Gateway {
    fn charge(&self, subtotal: u64) -> u64 {
        subtotal + self.fee
    }
}

#[factory(key = "card")]
fn card_gateway() -> Gateway {
    Gateway { fee: 3 }
}

#[factory(key = 7)]
fn wallet_gateway() -> Gateway {
    Gateway { fee: 7 }
}

#[injectable]
struct Checkout {
    #[inject("card")]
    card: Gateway,
    #[inject(7)]
    wallet: Gateway,
    #[inject("card")]
    card_port: dyn PaymentGateway,
    #[inject(7)]
    wallet_port: dyn PaymentGateway,
    #[inject("card")]
    optional_card: Option<Gateway>,
    #[inject(7)]
    optional_wallet: Option<dyn PaymentGateway>,
    #[inject("7")]
    wrong_named: Option<dyn PaymentGateway>,
    #[inject(99)]
    wrong_indexed: Option<Gateway>,
}

struct SyncQuote([u64; 4]);
struct AsyncQuote([u64; 4]);

#[factory]
fn sync_quote(
    #[inject("card")] card: Gateway,
    #[inject(7)] wallet: dyn PaymentGateway,
    #[inject("card")] optional_card: Option<dyn PaymentGateway>,
    #[inject(7)] optional_wallet: Option<Gateway>,
    #[inject("7")] wrong_named: Option<Gateway>,
    #[inject(99)] wrong_indexed: Option<dyn PaymentGateway>,
) -> SyncQuote {
    assert!(wrong_named.is_none() && wrong_indexed.is_none());
    SyncQuote([
        card.charge(100),
        wallet.charge(100),
        optional_card.unwrap().charge(100),
        optional_wallet.unwrap().charge(100),
    ])
}

#[factory]
async fn async_quote(
    #[inject("card")] card: dyn PaymentGateway,
    #[inject(7)] wallet: Gateway,
    #[inject("card")] optional_card: Option<Gateway>,
    #[inject(7)] optional_wallet: Option<dyn PaymentGateway>,
    #[inject("7")] wrong_named: Option<dyn PaymentGateway>,
    #[inject(99)] wrong_indexed: Option<Gateway>,
) -> AsyncQuote {
    tokio::task::yield_now().await;
    assert!(wrong_named.is_none() && wrong_indexed.is_none());
    AsyncQuote([
        card.charge(200),
        wallet.charge(200),
        optional_card.unwrap().charge(200),
        optional_wallet.unwrap().charge(200),
    ])
}

#[tokio::test]
async fn literal_keys_preserve_payment_routes_in_fields_and_factory_parameters() {
    let provider = ServiceProvider::build().await.unwrap();
    let (checkout, sync, asynchronous) = tokio::join!(
        get_required_service!(provider, Checkout),
        get_required_service!(provider, SyncQuote),
        get_required_service!(provider, AsyncQuote),
    );
    let checkout = checkout.unwrap();
    assert_eq!(checkout.card.charge(100), 103);
    assert_eq!(checkout.wallet.charge(100), 107);
    assert_eq!(checkout.card_port.charge(100), 103);
    assert_eq!(checkout.wallet_port.charge(100), 107);
    assert_eq!(checkout.optional_card.as_ref().unwrap().charge(100), 103);
    assert_eq!(checkout.optional_wallet.as_ref().unwrap().charge(100), 107);
    assert!(checkout.wrong_named.is_none() && checkout.wrong_indexed.is_none());
    assert!(std::ptr::addr_eq(
        &*checkout.card as *const Gateway,
        &*checkout.card_port as *const dyn PaymentGateway,
    ));
    assert!(std::ptr::addr_eq(
        &*checkout.wallet as *const Gateway,
        &*checkout.wallet_port as *const dyn PaymentGateway,
    ));
    assert_eq!(sync.unwrap().0, [103, 107, 103, 107]);
    assert_eq!(asynchronous.unwrap().0, [203, 207, 203, 207]);
    provider.dispose_async().await.unwrap();
}
