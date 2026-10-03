//! 真实 DI 查询基准：两种 core 版本必须编译同一份业务源码。
//!
//! 容器构建、预热和显式关闭均在计时之外；缓存场景也排除首次实例化。
//! Transient 场景按批创建 scope，防止已发布实例持续堆积；每批只累计包含
//! 实例化的查询循环时间，不计入 scope 关闭。

use std::{hint::black_box, time::Instant};

use nestrs::{factory, injectable};
use nestrs_core::{ServiceKey, ServiceProvider, ServiceProviderRef};

#[injectable]
struct Catalog {
    #[value(17)]
    revision: usize,
}

trait PaymentGateway: Send + Sync {
    fn fee(&self) -> usize;
}

struct CardGateway(usize);

impl PaymentGateway for CardGateway {
    fn fee(&self) -> usize {
        self.0
    }
}

// 同时覆盖 factory-only concrete 和工具链自动 trait 投影。
#[factory(key = "card")]
fn card_gateway() -> CardGateway {
    CardGateway(23)
}

#[injectable(lifetime = Scoped)]
struct Cart {
    #[inject]
    catalog: Catalog,
}

#[injectable(lifetime = Transient)]
struct WorkUnit {
    #[value(31)]
    amount: usize,
}

#[injectable(lifetime = Transient)]
struct FanIn {
    // 相同类型的三个输入槽必须分别创建实例，不能按 provider 去重消费。
    #[inject]
    first: WorkUnit,
    #[inject]
    second: WorkUnit,
    #[inject]
    third: WorkUnit,
}

#[derive(Clone, Copy)]
enum Scenario {
    WarmSingleton,
    KeyedTrait,
    ScopedCache,
    TransientFanIn,
}

impl Scenario {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "warm-singleton" => Ok(Self::WarmSingleton),
            "keyed-trait" => Ok(Self::KeyedTrait),
            "scoped-cache" => Ok(Self::ScopedCache),
            "transient-fanin" => Ok(Self::TransientFanIn),
            _ => Err(format!("未知场景：{value}")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::WarmSingleton => "warm-singleton",
            Self::KeyedTrait => "keyed-trait",
            Self::ScopedCache => "scoped-cache",
            Self::TransientFanIn => "transient-fanin",
        }
    }

    fn expected_value(self) -> usize {
        match self {
            Self::WarmSingleton | Self::ScopedCache => 17,
            Self::KeyedTrait => 23,
            Self::TransientFanIn => 93,
        }
    }
}

struct Options {
    scenario: Scenario,
    iterations: usize,
    warmup: usize,
    runtime: String,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut options = Self {
            scenario: Scenario::WarmSingleton,
            iterations: 100_000,
            warmup: 2_000,
            runtime: "current-thread".to_owned(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or_else(|| format!("{flag} 需要参数"))?;
            match flag.as_str() {
                "--scenario" => options.scenario = Scenario::parse(&value)?,
                "--iterations" => {
                    options.iterations = value.parse().map_err(|_| "iterations 需要正整数")?;
                }
                "--warmup" => {
                    options.warmup = value.parse().map_err(|_| "warmup 需要正整数")?;
                }
                "--runtime" => options.runtime = value,
                _ => return Err(format!("未知参数：{flag}")),
            }
        }
        if options.iterations == 0 || options.warmup == 0 {
            return Err("iterations 和 warmup 均须大于 0".to_owned());
        }
        if !matches!(options.runtime.as_str(), "current-thread" | "multi-thread") {
            return Err("runtime 只支持 current-thread 或 multi-thread".to_owned());
        }
        Ok(options)
    }
}

// 每次查询都经过公开门面、冻结路由、协调器和 owner 缓存。保留真实异步开销，
// 不能用哈希表 get 微基准的结果代替本组数据。
async fn singleton_loop(provider: &ServiceProvider, iterations: usize) -> usize {
    let mut checksum = 0usize;
    for _ in 0..iterations {
        let service = provider.get_required_service::<Catalog>().await.unwrap();
        checksum = checksum.wrapping_add(black_box(service).revision);
    }
    black_box(checksum)
}

async fn keyed_loop(provider: &ServiceProvider, iterations: usize) -> usize {
    let mut checksum = 0usize;
    for _ in 0..iterations {
        // 公开 API 消费拥有名称的 ServiceKey；将字符串创建保留在测量中，
        // before/after 执行相同调用，报告不得把它误称为纯路由查找时间。
        let service = provider
            .get_required_keyed_service::<dyn PaymentGateway>(ServiceKey::Named("card".to_owned()))
            .await
            .unwrap();
        checksum = checksum.wrapping_add(black_box(service).fee());
    }
    black_box(checksum)
}

async fn scoped_loop(provider: ServiceProviderRef<'_>, iterations: usize) -> usize {
    let mut checksum = 0usize;
    for _ in 0..iterations {
        let service = provider.get_required_service::<Cart>().await.unwrap();
        checksum = checksum.wrapping_add(black_box(service).catalog.revision);
    }
    black_box(checksum)
}

async fn fanin_loop(provider: ServiceProviderRef<'_>, iterations: usize) -> usize {
    let mut checksum = 0usize;
    for _ in 0..iterations {
        let service = provider.get_required_service::<FanIn>().await.unwrap();
        let service = black_box(service);
        checksum = checksum
            .wrapping_add(service.first.amount + service.second.amount + service.third.amount);
    }
    black_box(checksum)
}

async fn transient_batches(provider: &ServiceProvider, iterations: usize) -> (u128, usize) {
    let mut remaining = iterations;
    let mut elapsed_ns = 0;
    let mut checksum = 0usize;
    while remaining != 0 {
        // 一批最多 2,048 个实例，scope 收纳它们的 lease；批间完整关闭后再开始。
        let count = remaining.min(512);
        let scope = provider.create_scope();
        // create_scope 只投递注册命令。先完成一次无关 Singleton 查询，确认
        // 协调器已处理 scope 注册，避免把异步注册工作混入计时区间。
        black_box(
            scope
                .service_provider()
                .get_required_service::<Catalog>()
                .await
                .unwrap(),
        );
        let start = Instant::now();
        checksum = checksum.wrapping_add(fanin_loop(scope.service_provider(), count).await);
        elapsed_ns += start.elapsed().as_nanos();
        scope.dispose_async().await.unwrap();
        remaining -= count;
    }
    (elapsed_ns, checksum)
}

async fn measure(options: &Options) -> (u128, usize) {
    let provider = ServiceProvider::build().await.unwrap();
    let result = match options.scenario {
        Scenario::WarmSingleton => {
            singleton_loop(&provider, options.warmup).await;
            let start = Instant::now();
            let checksum = singleton_loop(&provider, options.iterations).await;
            (start.elapsed().as_nanos(), checksum)
        }
        Scenario::KeyedTrait => {
            keyed_loop(&provider, options.warmup).await;
            let start = Instant::now();
            let checksum = keyed_loop(&provider, options.iterations).await;
            (start.elapsed().as_nanos(), checksum)
        }
        Scenario::ScopedCache => {
            let scope = provider.create_scope();
            scoped_loop(scope.service_provider(), options.warmup).await;
            let start = Instant::now();
            let checksum = scoped_loop(scope.service_provider(), options.iterations).await;
            let elapsed_ns = start.elapsed().as_nanos();
            scope.dispose_async().await.unwrap();
            (elapsed_ns, checksum)
        }
        Scenario::TransientFanIn => {
            transient_batches(&provider, options.warmup).await;
            transient_batches(&provider, options.iterations).await
        }
    };
    provider.dispose_async().await.unwrap();
    result
}

fn main() {
    let options = Options::parse().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let runtime = if options.runtime == "current-thread" {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    };
    let (elapsed_ns, checksum) = runtime.block_on(measure(&options));
    assert_eq!(
        checksum,
        options
            .iterations
            .wrapping_mul(options.scenario.expected_value())
    );
    println!(
        "{{\"scenario\":\"{}\",\"runtime\":\"{}\",\"iterations\":{},\"warmup\":{},\"elapsed_ns\":{},\"ns_per_op\":{:.6},\"checksum\":{}}}",
        options.scenario.name(),
        options.runtime,
        options.iterations,
        options.warmup,
        elapsed_ns,
        elapsed_ns as f64 / options.iterations as f64,
        checksum,
    );
}
