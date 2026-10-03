//! 命令行是输入/输出适配层：解析用户订单，展示应用返回的结果。
use std::num::NonZeroUsize;

use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use nestrs_core::InitializationMode;

use crate::{
    application::{RunOptions, RunReport},
    domain::{CheckoutRequest, PaymentMethod},
};

/// 一次命令调用的容器选项与待执行业务操作。
pub(crate) struct Invocation {
    pub options: RunOptions,
    pub command: CheckoutCommand,
}

/// 单笔输入和固定样例共用应用层的请求处理流程。
pub(crate) enum CheckoutCommand {
    PlaceOrder(CheckoutRequest),
    Sample,
}

impl CheckoutCommand {
    /// 样例内预设业务拒绝不应使整个正常演示流程失败。
    pub fn is_sample(&self) -> bool {
        matches!(self, Self::Sample)
    }

    /// 将命令消费为实际订单输入，样例包含成功、拒付和库存不足。
    pub fn requests(self) -> Vec<CheckoutRequest> {
        match self {
            Self::PlaceOrder(request) => vec![request],
            Self::Sample => [
                ("Alice", 1, PaymentMethod::Card, "approved"),
                ("Bob", 2, PaymentMethod::Wallet, "approved"),
                ("Carol", 1, PaymentMethod::Card, "declined"),
                ("Dave", 99, PaymentMethod::Wallet, "approved"),
            ]
            .into_iter()
            .map(|(customer, quantity, payment, token)| CheckoutRequest {
                customer: customer.into(),
                sku: "KEYBOARD".into(),
                quantity,
                payment,
                payment_token: token.into(),
            })
            .collect(),
        }
    }
}

fn command() -> Command {
    let place_order = Command::new("place-order")
        .about("提交一笔订单；业务拒绝返回非零退出码")
        .args([
            Arg::new("customer")
                .long("customer")
                .required(true)
                .value_name("NAME")
                .help("客户名称"),
            Arg::new("sku")
                .long("sku")
                .default_value("KEYBOARD")
                .help("商品编号"),
            Arg::new("quantity")
                .long("quantity")
                .default_value("1")
                .value_parser(value_parser!(u32))
                .help("购买数量，单价由服务端目录计算"),
            Arg::new("payment")
                .long("payment")
                .default_value("card")
                .value_parser(["card", "wallet"])
                .help("支付渠道"),
            Arg::new("payment-token")
                .long("payment-token")
                .default_value("approved")
                .help("本地模拟凭据，declined 表示拒绝支付"),
        ]);

    Command::new("checkout")
        .about("本地电商结账应用：处理订单、库存预留、支付与审计")
        .disable_help_flag(true)
        .disable_help_subcommand(true)
        .subcommand_help_heading("命令")
        .help_template("{about}\n\n用法：{usage}\n\n{all-args}{after-help}")
        .args([
            Arg::new("help")
                .short('h')
                .long("help")
                .global(true)
                .action(ArgAction::Help)
                .help("显示帮助"),
            Arg::new("eager")
                .long("eager")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("启动时预热 Singleton；默认 Lazy"),
            Arg::new("scope-initialization")
                .long("scope-initialization")
                .global(true)
                .default_value("lazy")
                .value_parser(["lazy", "eager"])
                .help("创建请求作用域时采用的初始化策略；默认 lazy"),
            Arg::new("max-concurrency")
                .long("max-concurrency")
                .global(true)
                .value_name("N")
                .default_value("4")
                .value_parser(value_parser!(NonZeroUsize))
                .help("整个容器的构造并发上限，必须大于 0"),
        ])
        .subcommand(place_order)
        .subcommand(Command::new("sample").about("提交四笔样例订单，观察成功、拒付和库存不足"))
        .after_help("所有数据、连接和支付均为本地模拟。每次启动使用独立的内存数据。\n依赖图：cargo nestrs graph -p nestrs-di-example")
}

/// 解析参数；未给子命令时打印帮助并返回 None，避免创建容器。
pub(crate) fn parse() -> Result<Option<Invocation>, clap::Error> {
    let mut command = command();
    let matches = command.clone().try_get_matches()?;
    if matches.subcommand().is_none() {
        command
            .print_help()
            .map_err(|error| clap::Error::raw(clap::error::ErrorKind::Io, error))?;
        println!();
        return Ok(None);
    }
    Ok(Some(invocation(&matches)))
}

fn invocation(matches: &ArgMatches) -> Invocation {
    let options = RunOptions {
        initialization: if matches.get_flag("eager") {
            InitializationMode::Eager
        } else {
            InitializationMode::Lazy
        },
        scope_initialization: match matches
            .get_one::<String>("scope-initialization")
            .unwrap()
            .as_str()
        {
            "eager" => InitializationMode::Eager,
            _ => InitializationMode::Lazy,
        },
        max_concurrent_activations: *matches.get_one::<NonZeroUsize>("max-concurrency").unwrap(),
    };
    let command = match matches.subcommand().unwrap() {
        ("sample", _) => CheckoutCommand::Sample,
        ("place-order", order) => CheckoutCommand::PlaceOrder(CheckoutRequest {
            customer: order.get_one::<String>("customer").unwrap().clone(),
            sku: order.get_one::<String>("sku").unwrap().clone(),
            quantity: *order.get_one::<u32>("quantity").unwrap(),
            payment: match order.get_one::<String>("payment").unwrap().as_str() {
                "wallet" => PaymentMethod::Wallet,
                _ => PaymentMethod::Card,
            },
            payment_token: order.get_one::<String>("payment-token").unwrap().clone(),
        }),
        _ => unreachable!("Clap validates the subcommand"),
    };
    Invocation { options, command }
}

pub(crate) fn print_report(report: &RunReport) {
    for response in &report.responses {
        match &response.result {
            Ok(receipt) => println!("[订单] {receipt}"),
            Err(error) => println!("[拒绝] 客户 {}：{error}", response.customer),
        }
    }
    let total_cents: u64 = report.orders.iter().map(|order| order.total_cents).sum();
    println!(
        "[结果] 成功订单 {} 笔；剩余库存 {} 件；成交金额 {}.{:02} 元；审计 {} 条",
        report.orders.len(),
        report.remaining_stock,
        total_cents / 100,
        total_cents % 100,
        report.audit_entries,
    );
}
