//! 以完整图的传递可达性作为独立语义 oracle，不复写生产 Kahn/桶/置换算法。

use std::sync::{Arc, Mutex};

use ahash::AHashMap;

use super::CleanupOrder;
use crate::{
    ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, DependencyLease, ErasedService, InputSlot,
        LazyInputPlan, ReleaseDomain, project_required,
    },
    graph::{
        AbsentInput, CompiledDependency, CompiledNode, Constructor, DependencyInput, NodePolicy,
        ValidatedGraph,
    },
    runtime::owner::{OwnerData, Published},
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};

fn must_not_construct(_: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    panic!("关闭排序不能调用 constructor");
}

/// Floyd-Warshall 只用布尔可达性描述语义，与生产的反向邻接计数无关。
fn reachability(edges: &[Vec<bool>]) -> Vec<Vec<bool>> {
    let mut reachable = edges.to_vec();
    for via in 0..edges.len() {
        for consumer in 0..edges.len() {
            for dependency in 0..edges.len() {
                reachable[consumer][dependency] |=
                    reachable[consumer][via] && reachable[via][dependency];
            }
        }
    }
    reachable
}

/// 尚存实例的任何传递消费者都阻止它被清理；无此阻塞时优先最后发布的实例。
fn expected_cleanup(publications: &[usize], reachable: &[Vec<bool>]) -> Vec<usize> {
    let mut live = vec![true; publications.len()];
    let mut expected = Vec::with_capacity(publications.len());
    while expected.len() != publications.len() {
        let selected = (0..publications.len())
            .rev()
            .find(|&candidate| {
                live[candidate]
                    && !(0..publications.len()).any(|consumer| {
                        live[consumer] && reachable[publications[consumer]][publications[candidate]]
                    })
            })
            .expect("无环图中的剩余实例至少有一个不受消费者阻塞");
        live[selected] = false;
        expected.push(selected);
    }
    expected
}

fn frozen_graph(edges: &[Vec<bool>]) -> ValidatedGraph {
    let source = ServiceSource::new(file!(), line!(), 1);
    let identifiers: Vec<_> = (0..edges.len())
        .map(|provider| {
            ServiceIdentifier::new(
                Some(ServiceKey::Indexed(provider)),
                ServiceType::create::<usize>(),
            )
        })
        .collect();
    let nodes = edges
        .iter()
        .enumerate()
        .map(|(provider, dependencies)| {
            let mut inputs = Vec::new();
            for (target, &present) in dependencies.iter().enumerate() {
                if !present {
                    continue;
                }
                // 同一目标占两个真实槽位；覆盖仅普通、仅 lazy 和二者混用。
                // 反向邻接只含一个消费者，不能因重复输入多减一次入度。
                let kinds = match (provider + target) % 3 {
                    0 => [false, false],
                    1 => [true, true],
                    _ => [false, true],
                };
                for lazy in kinds {
                    let slot = InputSlot::new(inputs.len());
                    inputs.push(CompiledDependency {
                        slot,
                        requested: identifiers[target].clone(),
                        optional: false,
                        input: if lazy {
                            DependencyInput::Lazy {
                                plan: Arc::new(LazyInputPlan {
                                    provider: target,
                                    consumer: identifiers[provider].clone(),
                                    source,
                                    label: None,
                                    input: slot,
                                    project: project_required::<usize>,
                                }),
                            }
                        } else {
                            DependencyInput::Immediate {
                                target,
                                project: project_required::<usize>,
                            }
                        },
                        label: None,
                    });
                }
            }
            inputs.push(CompiledDependency {
                slot: InputSlot::new(inputs.len()),
                requested: identifiers[provider].clone(),
                optional: true,
                input: DependencyInput::Absent(AbsentInput::Lazy),
                label: None,
            });
            CompiledNode {
                identifier: identifiers[provider].clone(),
                common: NodePolicy {
                    lifetime: ServiceLifetime::Transient,
                    lazy: None,
                    source,
                    cleanup: None,
                },
                dependencies: inputs,
                constructor: Constructor::Class(must_not_construct),
                requires_scope: false,
            }
        })
        .collect();
    let reachable = reachability(edges);
    let mut topological_order: Vec<_> = (0..edges.len()).collect();
    topological_order.sort_by_key(|&provider| reachable[provider].iter().filter(|&&v| v).count());
    ValidatedGraph {
        nodes,
        routes: AHashMap::new(),
        topological_order,
        dependents: (0..edges.len())
            .map(|target| {
                (0..edges.len())
                    .filter(|&consumer| edges[consumer][target])
                    .collect()
            })
            .collect(),
    }
}

#[test]
fn every_four_provider_dag_and_short_publication_history_obeys_live_consumers() {
    const PROVIDERS: usize = 4;
    let domain = ReleaseDomain::new();
    // 四个固定的不同 lease 标记原始发布序号；各案例只克隆令牌，不调用构造器。
    let instances: Vec<_> = (0..PROVIDERS)
        .map(|sequence| DependencyLease::new(ErasedService::new(sequence), vec![], domain.clone()))
        .collect();
    let possible_edges: Vec<_> = (0..PROVIDERS)
        .flat_map(|consumer| {
            (0..PROVIDERS)
                .filter(move |&target| target != consumer)
                .map(move |target| (consumer, target))
        })
        .collect();
    let mut order = CleanupOrder::default();
    let mut cases = 0;
    let mut dags = 0;
    for mask in 0..(1 << possible_edges.len()) {
        let mut edges = vec![vec![false; PROVIDERS]; PROVIDERS];
        for (bit, &(consumer, target)) in possible_edges.iter().enumerate() {
            edges[consumer][target] = mask & (1 << bit) != 0;
        }
        let reachable = reachability(&edges);
        if (0..PROVIDERS).any(|provider| reachable[provider][provider]) {
            continue;
        }
        dags += 1;
        let graph = frozen_graph(&edges);
        for length in 0..=PROVIDERS {
            for mut encoding in 0..PROVIDERS.pow(length as u32) {
                let publications: Vec<_> = (0..length)
                    .map(|_| {
                        let provider = encoding % PROVIDERS;
                        encoding /= PROVIDERS;
                        provider
                    })
                    .collect();
                let expected = expected_cleanup(&publications, &reachable);
                let mut journal: Vec<_> = publications
                    .iter()
                    .enumerate()
                    .map(|(sequence, &provider)| Published {
                        provider,
                        lease: instances[sequence].clone(),
                    })
                    .collect();
                order.order(&mut journal, &graph);
                let actual: Vec<_> = journal
                    .iter()
                    .rev()
                    .map(|entry| {
                        let sequence = instances
                            .iter()
                            .position(|lease| lease.ptr_eq(&entry.lease))
                            .unwrap();
                        assert_eq!(entry.provider, publications[sequence]);
                        sequence
                    })
                    .collect();
                assert_eq!(
                    actual, expected,
                    "edges={edges:?}, journal={publications:?}"
                );
                cases += 1;
            }
        }
    }
    assert_eq!(dags, 543);
    assert_eq!(cases, 185_163);
}

#[test]
fn owners_reuse_ordering_without_retaining_or_releasing_each_others_instances() {
    struct Tracked {
        sequence: usize,
        drops: Arc<Mutex<Vec<usize>>>,
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.drops.lock().unwrap().push(self.sequence);
        }
    }

    let edges = vec![
        vec![false, true, false, false],
        vec![false, false, true, false],
        vec![false, false, false, false],
        vec![false, false, true, false],
    ];
    let graph = frozen_graph(&edges);
    let reachable = reachability(&edges);
    let histories: &[(&[usize], &[usize])] = &[
        (&[0, 2], &[0, 1]), // 中间 provider 1 未实例化，仍须先清理 0，再清理 2。
        (&[2, 0, 3, 0, 2, 3], &[5, 3, 2, 1, 4, 0]), // 重复 occurrence 与逆发布次序竞争。
        (&[], &[]),
        (&[1], &[0]),
        (&[0, 3], &[1, 0]), // 无活着的传递消费者，按逆发布序号清理。
        (&[1, 2, 0, 3, 2, 1, 0, 3], &[7, 6, 3, 2, 5, 0, 4, 1]),
        (&[2, 0], &[1, 0]),
    ];
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut order = CleanupOrder::default();
    for (id, &(publications, expected)) in histories.iter().enumerate() {
        let owner = OwnerData::new(id as u64, commands.downgrade());
        let domain = ReleaseDomain::new();
        let drops = Arc::new(Mutex::new(Vec::new()));
        for (sequence, &provider) in publications.iter().enumerate() {
            owner.publish(
                provider,
                DependencyLease::new(
                    ErasedService::new(Tracked {
                        sequence,
                        drops: drops.clone(),
                    }),
                    vec![],
                    domain.clone(),
                ),
            );
        }
        owner.order_cleanup(&graph, &mut order);
        assert!(drops.lock().unwrap().is_empty(), "重排不得释放实例");
        assert_eq!(expected_cleanup(publications, &reachable), expected);
        let mut actual = Vec::new();
        while let Some(entry) = owner.next_cleanup() {
            let pointer = entry.lease.pointer::<Tracked>().unwrap();
            // SAFETY: entry 的强 lease 在读取期间保活准确的 Tracked 实例。
            actual.push(unsafe { pointer.as_ref() }.sequence);
            drop(entry);
        }
        assert_eq!(actual, expected);
        assert_eq!(
            drops.lock().unwrap().as_slice(),
            expected,
            "工作区不能额外保活实例"
        );
    }
    order.reclaim();
    order.order(&mut [], &frozen_graph(&[]));
}
