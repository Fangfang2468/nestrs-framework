#![allow(dead_code)]

use nestrs::bind;

trait Port: Send + Sync {}
struct Service;

// bind 仅用于观察隐藏的内部 ABI 诊断，不是推荐业务 API。
// 故意声明投影却不给 Service 声明 provider；投影本身不会创建实例。
#[bind]
impl Port for Service {}

fn main() {}
