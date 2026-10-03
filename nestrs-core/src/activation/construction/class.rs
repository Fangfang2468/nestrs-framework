//! Class provider 构造 adapter 的 ABI。

use super::{ConstructionError, ConstructionInputs};
use crate::activation::erased_service::ErasedService;

/// 所有 class provider 构造 adapter 的统一函数签名。
///
/// adapter 只能取得完整的 [`ConstructionInputs`]。生成实现必须先取出全部依赖并调用
/// `ensure_all_consumed()`，随后才执行用户构造及字段默认值，避免输入失败前的用户副作用。
pub type ClassConstructor = fn(ConstructionInputs) -> Result<ErasedService, ConstructionError>;
