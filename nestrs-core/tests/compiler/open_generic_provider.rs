use nestrs::injectable;
use std::marker::PhantomData;

// 此测试 crate 中只声明这两个开放泛型服务。
#[injectable]
struct A<T> {
    marker: PhantomData<T>,
}

#[injectable]
struct B<T> {
    #[inject]
    a: A<T>,
}

#[test]
fn open_generic_chain_without_a_closed_root_contributes_no_execution_nodes() {
    // 工具链只保存开放泛型的生成能力，未出现闭合查询或消费依赖时不能猜测 T。
    // 闭合 B<u32> -> A<u32> 的构造链在 generic_provider 独立入口中验证。
    let graph = &crate::graph::plan::load().graph;
    assert!(graph.nodes.is_empty());
    assert!(graph.routes.is_empty());
    assert!(graph.topological_order.is_empty());
}
