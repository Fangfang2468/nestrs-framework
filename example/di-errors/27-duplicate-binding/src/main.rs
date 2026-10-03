#![allow(dead_code)]

use nestrs::{bind, injectable};

trait Port: Send + Sync {}

#[injectable]
struct Service;

// bind 仅用于观察隐藏的内部 ABI 诊断，不是推荐业务 API。
// 正常业务只写 impl，由工具自动绑定；此处故意重复声明同一个 pair。
#[bind]
#[bind]
impl Port for Service {}

fn main() {}
