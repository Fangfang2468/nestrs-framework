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
    // 故意错误：optional 只允许缺席，不能替两个同 key 的候选做选择。
    #[inject]
    sender: Option<dyn MessagePort>,
}

fn main() {}
