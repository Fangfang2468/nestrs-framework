//! Class provider 构造 adapter 的 ABI。

use super::{ConstructionError, ConstructionInputs};
use crate::activation::erased_service::ErasedService;

/// 所有 class provider 构造 adapter 的统一函数签名。
///
/// adapter 只能取得完整的 [`ConstructionInputs`]。宏生成的实现必须在构造实例后调用
/// `ensure_all_consumed()`，以拒绝 descriptor 与 adapter 漂移留下的槽位。
pub type ClassConstructor = fn(ConstructionInputs) -> Result<ErasedService, ConstructionError>;
