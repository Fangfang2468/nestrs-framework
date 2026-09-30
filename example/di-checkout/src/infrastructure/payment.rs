use nestrs::factory;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use thiserror::Error;

use crate::{
    config::AppConfig,
    domain::{ChargeFuture, CheckoutError, PaymentGateway},
};

struct PaymentClient {
    id: usize,
    channel: &'static str,
    merchant: String,
}

static NEXT_PAYMENT_REFERENCE: AtomicUsize = AtomicUsize::new(1);

impl PaymentGateway for PaymentClient {
    fn channel(&self) -> &'static str {
        self.channel
    }
    #[cfg(test)]
    fn id(&self) -> usize {
        self.id
    }

    fn charge<'request>(
        &'request self,
        total_cents: u64,
        token: &'request str,
    ) -> ChargeFuture<'request> {
        Box::pin(async move {
            // 模拟支付请求的异步 I/O，不连接外部支付系统，也不打印支付凭证。
            tokio::time::sleep(Duration::from_millis(25)).await;
            if token == "declined" {
                crate::observe::event(format!("业务支付拒绝 [{}]，库存预留将回滚", self.channel));
                return Err(CheckoutError::PaymentDeclined);
            }
            let number = NEXT_PAYMENT_REFERENCE.fetch_add(1, Ordering::Relaxed);
            let reference = format!("{}-{number:04}", self.channel);
            crate::observe::event(format!(
                "业务支付成功 [{}] merchant={} amount={total_cents} 分 reference={reference}",
                self.channel, self.merchant,
            ));
            Ok(reference)
        })
    }
}

/// Debug 中保留明确原因，经过现有 factory 错误映射后仍能看到注入故障的来源。
#[derive(Debug, Error)]
#[error("支付客户端 {channel} 初始化失败：{reason}")]
struct PaymentInitializationError {
    channel: &'static str,
    reason: &'static str,
}

#[factory(key = "card", cleanup = "cleanup_card")]
async fn card(config: AppConfig) -> Result<PaymentClient, PaymentInitializationError> {
    crate::observe::event("factory start PaymentClient[card] (100 ms)");
    tokio::time::sleep(Duration::from_millis(100)).await;
    if config.fail_payment_initialization {
        let error = PaymentInitializationError {
            channel: "card",
            reason: "NESTRS_EXAMPLE_FAIL_PAYMENT=1：演示支付客户端初始化失败",
        };
        crate::observe::event(format!("factory end PaymentClient[card] (failed: {error})"));
        return Err(error);
    }
    let client = PaymentClient {
        id: crate::observe::created("PaymentClient[card]"),
        channel: "card",
        merchant: config.merchant_name.clone(),
    };
    crate::observe::event(format!("factory end PaymentClient[card] #{}", client.id));
    Ok(client)
}

#[factory(key = "wallet", cleanup = "cleanup_wallet")]
async fn wallet(config: AppConfig) -> PaymentClient {
    crate::observe::event("factory start PaymentClient[wallet] (120 ms)");
    tokio::time::sleep(Duration::from_millis(120)).await;
    let client = PaymentClient {
        id: crate::observe::created("PaymentClient[wallet]"),
        channel: "wallet",
        merchant: config.merchant_name.clone(),
    };
    crate::observe::event(format!("factory end PaymentClient[wallet] #{}", client.id));
    client
}

async fn cleanup_card() {
    crate::observe::event("cleanup hook PaymentClient[card]（零参数，仅记录钩子调用）");
}

async fn cleanup_wallet() {
    crate::observe::event("cleanup hook PaymentClient[wallet]（零参数，仅记录钩子调用）");
}

impl Drop for PaymentClient {
    fn drop(&mut self) {
        crate::observe::event(format!("drop PaymentClient[{}] #{}", self.channel, self.id));
    }
}
