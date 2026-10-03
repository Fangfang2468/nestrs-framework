#![allow(dead_code)]

use nestrs::injectable;

trait MessagePort: Send + Sync {}

#[injectable]
struct EmailSender;

#[injectable]
struct SmsSender;

impl MessagePort for EmailSender {}
impl MessagePort for SmsSender {}

#[injectable]
struct NotificationService {
    // 故意错误：延迟构造不延迟候选选择；编译期仍须唯一确定 trait 的实现。
    #[inject]
    #[lazy]
    sender: dyn MessagePort,
}

fn main() {}
