//! 门面与运行时共用的错误模型。
//!
//! 图结构错误由工具链在编译期报告；本模块表达编译计划缺失、运行环境、
//! 实例初始化和关闭失败。
//! 错误类型位于独立的内部模块，避免 runtime 为了报告错误而反向依赖 facade。
//! 公开类型仍从 crate 根导出，错误文本、共享失败记录和依赖路径顺序保持不变。

use std::{fmt, sync::Arc};

use crate::service::{ServiceIdentifier, ServiceSource};

/// 编译计划缺失、运行环境或启动预热失败。静态图错误由工具链在编译期报告。
/// Lazy 默认下显式 #[lazy(false)] 的初始化失败也属于启动失败。
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("服务容器缺少编译计划；请使用 cargo nestrs check/build/run/test 构建应用")]
    CompilerPlanUnavailable,
    #[error("构建服务容器需要当前 Tokio runtime")]
    RuntimeUnavailable,
    #[error("服务容器预热失败: {error}; 关闭结果: {dispose_error:?}")]
    Initialization {
        #[source]
        error: ResolveError,
        dispose_error: Option<DisposeError>,
    },
}

struct ResolveFailure {
    // 同一个原始故障的所有路径共享详情，向上追加路径不复制整段文本。
    detail: Arc<str>,
    path: Option<Arc<FailureFrame>>,
}

/// 一次依赖传播只新增当前服务的路径帧，后续链段继续与缓存中的原始错误共享。
/// 链的显示和释放都显式迭代，不能依赖递归 Display 或 Arc 的递归级联析构。
struct FailureFrame {
    identifier: ServiceIdentifier,
    source: ServiceSource,
    parent: Option<Arc<FailureFrame>>,
}

impl Drop for FailureFrame {
    fn drop(&mut self) {
        // 缓存或其他请求可能仍持有后缀；只拆除当前独占的前缀。
        // 每次先取走 parent，再让当前帧离开作用域，因此深路径不会递归调用 Drop。
        let mut parent = self.parent.take();
        while let Some(frame) = parent {
            let Some(mut frame) = Arc::into_inner(frame) else {
                break;
            };
            parent = frame.parent.take();
        }
    }
}

/// 获取或激活失败，保留失败原因和带 key、源码位置的依赖路径。
///
/// Singleton/Scoped 的失败被缓存，后续调用共享原始失败记录。
#[derive(Clone)]
pub struct ResolveError(Arc<ResolveFailure>);

impl ResolveError {
    pub(crate) fn new(detail: String) -> Self {
        Self(Arc::new(ResolveFailure {
            detail: detail.into(),
            path: None,
        }))
    }

    pub(crate) fn closed() -> Self {
        Self::new("服务 owner 已关闭或正在关闭".to_owned())
    }

    pub(crate) fn construction(
        identifier: &ServiceIdentifier,
        source: ServiceSource,
        detail: String,
    ) -> Self {
        Self::dependency(identifier, source, Self::new(detail))
    }

    pub(crate) fn dependency(
        identifier: &ServiceIdentifier,
        source: ServiceSource,
        dependency: Self,
    ) -> Self {
        // 每个消费者拥有自己的路径头，原始故障及共享后缀不被修改。
        // 这样同一次 Singleton 失败可以安全地报告给多条不同的请求路径。
        Self(Arc::new(ResolveFailure {
            detail: dependency.0.detail.clone(),
            path: Some(Arc::new(FailureFrame {
                identifier: identifier.clone(),
                source,
                parent: dependency.0.path.clone(),
            })),
        }))
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "服务解析失败: {}", self.0.detail)?;
        let mut current = self.0.path.as_deref();
        while let Some(frame) = current {
            let FailureFrame {
                identifier,
                source,
                parent,
            } = frame;
            current = parent.as_deref();
            write!(
                formatter,
                "\n  {} key={:?} ({}:{}:{})",
                identifier.service_type.name,
                identifier.service_key,
                source.file,
                source.line,
                source.column
            )?;
        }
        Ok(())
    }
}
impl fmt::Debug for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Debug 也走迭代显示；否则 BuildError 的派生 Debug 可能重新引入深链递归。
        fmt::Display::fmt(self, formatter)
    }
}
impl std::error::Error for ResolveError {}

/// 所有 cleanup 都处理完成后汇总的关闭错误。
#[derive(Debug, Clone, thiserror::Error)]
#[error("服务容器关闭失败: {failures:?}")]
pub struct DisposeError {
    failures: Arc<Vec<String>>,
}
impl DisposeError {
    pub(crate) fn coordinator_stopped() -> Self {
        Self::new(vec![
            "Tokio 协调任务已经停止；异步 cleanup 未确认完成".to_owned(),
        ])
    }

    pub(crate) fn new(failures: Vec<String>) -> Self {
        Self {
            failures: Arc::new(failures),
        }
    }

    /// 每个失败 cleanup 的诊断。其余实例仍会继续清理。
    pub fn failures(&self) -> &[String] {
        &self.failures
    }
}

#[cfg(test)]
#[path = "../tests/unit/error.rs"]
mod tests;
