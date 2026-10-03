#![allow(dead_code)]

use nestrs::{factory, injectable};

struct Database;
struct Queue;

// 错误 1、2：同一个服务中的两个必选字段分别缺少创建声明。
#[injectable]
struct OrderService {
    #[inject]
    database: Database,
    #[inject]
    queue: Queue,
}

#[injectable(key = "sandbox")]
struct EmailClient;

// 错误 3：已有 EmailClient，但消费方请求的是另一个 key。
#[injectable]
struct Mailer {
    #[inject("live")]
    client: EmailClient,
}

trait PaymentGateway: Send + Sync {}

#[injectable]
struct CardGateway;
impl PaymentGateway for CardGateway {}

#[injectable]
struct BankGateway;
impl PaymentGateway for BankGateway {}

// 错误 4：两个实现都可用，没有唯一 primary；不应再派生一条缺失错误。
#[injectable]
struct Checkout {
    #[inject]
    gateway: dyn PaymentGateway,
}

struct ReportConfig;
struct ReportService;

// 错误 5：factory 参数也会与字段错误一起报告。
#[factory]
fn report_service(_config: ReportConfig) -> ReportService {
    ReportService
}

// 无需构建容器或执行用户逻辑，最终入口的编译就会报告上述 5 个错误。
fn main() {}
