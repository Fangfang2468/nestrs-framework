//! 结账用例：只通过已注入的业务协作者工作，不查询或保存容器。

mod context;
mod receipt;
mod service;

pub(crate) use service::CheckoutService;
#[cfg(test)]
pub(crate) use {context::RequestContext, receipt::ReceiptFormatter};
