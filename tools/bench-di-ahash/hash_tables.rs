//! DI 哈希表的独立微基准，不链接 core，也不绕过它的私有模块。
//!
//! 用法：hash_tables --hasher <std|ahash> --scenario <场景> --iterations <次数>
//! 每次执行输出一个 JSON 样本；多次采样、交替顺序和统计由外部 runner 负责。
//! 两个分支都使用标准库的 HashMap/HashSet，唯一变化是 BuildHasher。

use std::any::{TypeId, type_name};
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::hint::black_box;
use std::time::{Duration, Instant};

const TABLE_LEN: usize = 1024;
// 完整执行偶数轮，parent-churn 预热后恢复最初的键集合。
const WARMUP_ITERATIONS: usize = TABLE_LEN * 16;

// 只复制当前 core 路由键的形状与 derive Hash 的字段顺序。
// 这是模型，不是对 core 私有 ServiceIdentifier 的实际调用。
#[derive(Clone, PartialEq, Eq, Hash)]
enum ServiceKey {
    Named(String),
    Indexed(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ServiceType {
    type_id: TypeId,
    name: &'static str,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ServiceIdentifier {
    service_key: Option<ServiceKey>,
    service_type: ServiceType,
}

// 通过真实 TypeId 与 type_name 生成包含模块路径的类型名，避免只测短字符串。
mod application {
    pub mod checkout {
        pub struct CheckoutService;
        pub struct InventoryService;
        pub struct PaymentGateway;
        pub struct OrderRepository;
    }
}

fn service_type<T: 'static>() -> ServiceType {
    ServiceType {
        type_id: TypeId::of::<T>(),
        name: type_name::<T>(),
    }
}

fn route_keys() -> Vec<ServiceIdentifier> {
    use application::checkout::*;

    let types = [
        service_type::<CheckoutService>(),
        service_type::<InventoryService>(),
        service_type::<PaymentGateway>(),
        service_type::<OrderRepository>(),
    ];
    (0..TABLE_LEN)
        .map(|index| ServiceIdentifier {
            service_key: Some(if index % 2 == 0 {
                ServiceKey::Named(format!("tenant-{index:04}/checkout/payment"))
            } else {
                ServiceKey::Indexed(index)
            }),
            service_type: types[index % types.len()],
        })
        .collect()
}

/// 用固定置换统一两个实现的访问顺序，不把随机数生成成本算作哈希成本。
/// 哈希器本身仍各自使用默认随机种子，没有固定 seed 或关闭随机化。
fn lookup_order() -> Vec<usize> {
    (0..TABLE_LEN)
        .map(|index| (index * 613 + 97) & (TABLE_LEN - 1))
        .collect()
}

fn lookup<K: Eq + std::hash::Hash, S: BuildHasher>(
    map: &HashMap<K, usize, S>,
    keys: &[K],
    order: &[usize],
    iterations: usize,
) -> usize {
    let mut checksum = 0usize;
    for iteration in 0..iterations {
        let index = order[iteration & (TABLE_LEN - 1)];
        let value = map.get(black_box(&keys[index])).unwrap();
        checksum = checksum.wrapping_add(black_box(*value));
    }
    black_box(checksum)
}

fn lookup_sample<K: Eq + std::hash::Hash + Clone, S: BuildHasher + Default>(
    keys: Vec<K>,
    iterations: usize,
) -> (Duration, usize) {
    let mut map = HashMap::<K, usize, S>::with_capacity_and_hasher(TABLE_LEN, S::default());
    for (index, key) in keys.iter().enumerate() {
        map.insert(key.clone(), index);
    }
    let order = lookup_order();
    lookup(&map, &keys, &order, WARMUP_ITERATIONS);
    // 建表、分配 key、复制 String 和预热均在计时之前。
    let start = Instant::now();
    let checksum = lookup(&map, &keys, &order, iterations);
    (start.elapsed(), checksum)
}

fn churn<S: BuildHasher>(
    set: &mut HashSet<(usize, usize), S>,
    keys: &[[(usize, usize); 2]],
    order: &[usize],
    iterations: usize,
) -> usize {
    let mut checksum = 0usize;
    for iteration in 0..iterations {
        let index = order[iteration & (TABLE_LEN - 1)];
        let generation = (iteration / TABLE_LEN) & 1;
        // 每个槽位在两组预先生成的 parent/slot 对之间切换，集合始终为 1024 项。
        // 一次 iteration 是一次 remove 加一次 insert，因此报告计为两个操作。
        let removed = set.remove(black_box(&keys[index][generation]));
        let inserted = set.insert(black_box(keys[index][generation ^ 1]));
        checksum = checksum.wrapping_add(usize::from(removed) + usize::from(inserted));
    }
    black_box(checksum)
}

fn churn_sample<S: BuildHasher + Default>(iterations: usize) -> (Duration, usize) {
    let keys: Vec<_> = (0..TABLE_LEN)
        .map(|index| [(index, index % 8), (index + TABLE_LEN, index % 8)])
        .collect();
    let order = lookup_order();
    let mut set = HashSet::<_, S>::with_capacity_and_hasher(TABLE_LEN * 2, S::default());
    for pair in &keys {
        set.insert(pair[0]);
    }
    assert_eq!(
        churn(&mut set, &keys, &order, WARMUP_ITERATIONS),
        WARMUP_ITERATIONS * 2
    );
    let start = Instant::now();
    let checksum = churn(&mut set, &keys, &order, iterations);
    let elapsed = start.elapsed();
    assert_eq!(set.len(), TABLE_LEN);
    assert_eq!(checksum, iterations * 2);
    (elapsed, checksum)
}

fn sample<S: BuildHasher + Default>(scenario: &str, iterations: usize) -> (Duration, usize) {
    match scenario {
        "usize-lookup" => lookup_sample::<_, S>((0..TABLE_LEN).collect(), iterations),
        "route-lookup" => lookup_sample::<_, S>(route_keys(), iterations),
        "parent-churn" => churn_sample::<S>(iterations),
        _ => unreachable!("scenario 已在入口校验"),
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let usage = "用法：hash_tables --hasher <std|ahash> --scenario <usize-lookup|route-lookup|parent-churn> --iterations <次数>";
    let fail = || {
        eprintln!("{usage}");
        std::process::exit(2);
    };
    if args.len() != 6
        || args[0] != "--hasher"
        || args[2] != "--scenario"
        || args[4] != "--iterations"
        || !matches!(args[1].as_str(), "std" | "ahash")
        || !matches!(
            args[3].as_str(),
            "usize-lookup" | "route-lookup" | "parent-churn"
        )
    {
        fail();
    }
    let iterations = args[5]
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0 && *value <= usize::MAX / 2)
        .unwrap_or_else(|| {
            fail();
            unreachable!()
        });
    let (elapsed, checksum) = match args[1].as_str() {
        "std" => sample::<std::collections::hash_map::RandomState>(&args[3], iterations),
        "ahash" => sample::<ahash::RandomState>(&args[3], iterations),
        _ => unreachable!(),
    };
    let operations = iterations * if args[3] == "parent-churn" { 2 } else { 1 };
    let elapsed_ns = elapsed.as_nanos();
    let ns_per_operation = elapsed_ns as f64 / operations as f64;
    println!(
        "{{\"kind\":\"hash-table-model\",\"hasher\":\"{}\",\"scenario\":\"{}\",\"table_entries\":{},\"iterations\":{},\"operations\":{},\"elapsed_ns\":{},\"ns_per_op\":{:.6},\"checksum\":{}}}",
        args[1], args[3], TABLE_LEN, iterations, operations, elapsed_ns, ns_per_operation, checksum
    );
}
