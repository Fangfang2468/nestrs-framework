//! 将应用已经构造并检查过的内存层接入统一的来源加载流程。
//!
//! 适用于默认值、显式覆盖及测试输入；不读取外部状态，也不改变调用方已有的配置快照。

use crate::{ConfigError, ConfigLayer, ConfigSource, LoadContext};

/// 拥有应用提供的配置层的内存来源。
///
/// 配置值及逐节点来源信息按层保存；复制来源不会重新解析输入，也不会写回其他快照。
/// 默认调试输出仅标识来源类型，省略所有配置值。
#[derive(Clone)]
pub struct Memory {
    layer: ConfigLayer,
}

impl Memory {
    /// 接管已经构造成功的配置层。
    ///
    /// 本方法不加载文件、不解析环境变量；`String` 与待转换文本的区别由传入层保留。
    pub fn new(layer: ConfigLayer) -> Self {
        Self { layer }
    }
}

impl ConfigSource for Memory {
    /// 复制持有的层供本次构建独立合并，不依赖加载上下文或外部状态。
    fn load(&self, _context: &LoadContext) -> Result<ConfigLayer, ConfigError> {
        Ok(self.layer.clone())
    }
}

impl std::fmt::Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不递归格式化内部层，避免今后内部表示变化时意外输出原值。
        f.debug_struct("Memory").finish_non_exhaustive()
    }
}
