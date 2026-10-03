//! 门面与运行时共用的错误模型。
//!
//! 图结构错误由工具链在编译期报告；本模块表达编译计划缺失、运行环境、
//! 实例初始化和关闭失败。
//! 错误类型位于独立的内部模块，避免 runtime 为了报告错误而反向依赖 facade。
//! 公开类型仍从 crate 根导出，错误文本、共享失败记录和依赖路径顺序保持不变。

use std::{fmt, sync::Arc};

use crate::service::{ServiceIdentifier, ServiceSource};

/// 编译计划缺失、运行环境或 root 创建期初始化失败。静态图错误由工具链在编译期报告。
/// Lazy 默认下显式 `#[lazy(false)]` 的 Singleton 初始化失败也属于启动失败。
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// 应用没有经过工具链生成最终执行计划。
    #[error("服务容器缺少编译计划；请使用 cargo nestrs check/build/run/test 构建应用")]
    CompilerPlanUnavailable,

    /// 当前线程上下文没有可用的 Tokio runtime。
    #[error("构建服务容器需要当前 Tokio runtime")]
    RuntimeUnavailable,

    /// 创建期初始化失败，并保留关闭尚未交付 root 的结果。
    #[error("服务容器预热失败: {error}; 关闭结果: {dispose_error:?}")]
    Initialization {
        /// root 创建期初始化产生的原始解析失败。
        #[source]
        error: ResolveError,

        /// 清理尚未交付的 root 时发生的可选关闭失败。
        dispose_error: Option<DisposeError>,
    },
}

/// scope 登记或创建期初始化失败；返回前已请求并等待未交付 owner 的关闭结果。
///
/// 保留原始失败和可选的关闭失败，不把部分初始化的 scope 交给调用者。
/// 若协调器已停止，关闭错误表示无法确认异步 cleanup 完成，不作完成保证。
#[derive(Debug, thiserror::Error)]
#[error("服务作用域初始化失败: {error}; 关闭结果: {dispose_error:?}")]
pub struct ScopeBuildError {
    /// 创建 scope 时发生的注册或服务初始化错误。
    #[source]
    pub error: ResolveError,

    /// 关闭未交付 scope 时发生的可选清理错误。
    pub dispose_error: Option<DisposeError>,
}

/// 共享原始错误详情和一条消费者路径，避免传播时复制深链文本。
struct ResolveFailure {
    /// 同一故障所有路径共享的详情，向上追加路径时不复制文本。
    detail: Arc<str>,

    /// 当前消费者到原始故障的共享路径头。
    path: Option<Arc<FailureFrame>>,
}

/// 一次依赖传播只新增当前服务的路径帧，后续链段继续与缓存中的原始错误共享。
/// 链的显示和释放都显式迭代，不能依赖递归 Display 或 Arc 的递归级联析构。
struct FailureFrame {
    /// 本路径帧对应的服务与 key。
    identifier: ServiceIdentifier,

    /// 原始声明位置，供运行期失败诊断使用。
    source: ServiceSource,

    /// 下游失败的共享路径后缀。
    parent: Option<Arc<FailureFrame>>,
}

impl Drop for FailureFrame {
    /// 迭代拆除当前独占的错误路径前缀，保留其他请求共享的后缀。
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
/// Singleton/Scoped 的构造或依赖失败缓存到所属 owner 关闭，后续调用共享原始记录。
/// 查询被关闭状态或生命周期限制拒绝时，不会因此创建一个服务失败缓存。
#[derive(Clone)]
pub struct ResolveError(Arc<ResolveFailure>);

impl ResolveError {
    /// 创建没有依赖路径的原始解析失败。
    pub(crate) fn new(detail: String) -> Self {
        Self(Arc::new(ResolveFailure {
            detail: detail.into(),
            path: None,
        }))
    }

    /// 生成 owner 已停止接受新请求的解析错误。
    pub(crate) fn closed() -> Self {
        Self::new("服务 owner 已关闭或正在关闭".to_owned())
    }

    /// 将构造失败的详情归属到当前服务及其声明位置。
    pub(crate) fn construction(
        identifier: &ServiceIdentifier,
        source: ServiceSource,
        detail: String,
    ) -> Self {
        Self::dependency(identifier, source, Self::new(detail))
    }

    /// 追加消费者路径帧，继续共享下游故障详情与路径后缀。
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
    /// 按依赖路径迭代显示错误，避免深链造成递归格式化。
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
    /// 按依赖路径迭代显示错误，避免深链造成递归格式化。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Debug 也走迭代显示；否则 BuildError 的派生 Debug 可能重新引入深链递归。
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for ResolveError {}

/// 关闭期间的 cleanup、跟踪到的析构失败，或协调器未能交付关闭结果。
///
/// 正常关闭会汇总错误并继续清理其他实例；协调器提前停止时，不能确认 cleanup 已完成。
#[derive(Debug, Clone, thiserror::Error)]
#[error("服务容器关闭失败: {failures:?}")]
pub struct DisposeError {
    /// 完整关闭过程中汇总的失败文本。
    failures: Arc<Vec<String>>,
}

impl DisposeError {
    /// 表示协调器未能交付完整关闭结果。
    pub(crate) fn coordinator_stopped() -> Self {
        Self::new(vec![
            "Tokio 协调任务已经停止；异步 cleanup 未确认完成".to_owned(),
        ])
    }

    /// 共享本次关闭汇总的失败集合，供多个关闭等待者复用。
    pub(crate) fn new(failures: Vec<String>) -> Self {
        Self {
            failures: Arc::new(failures),
        }
    }

    /// 关闭失败的诊断，包括 cleanup、跟踪到的析构 panic 或协调器停止。
    /// 协调器仍运行时，个别 cleanup 失败不会阻止其余实例继续清理。
    pub fn failures(&self) -> &[String] {
        &self.failures
    }
}

#[cfg(test)]
#[path = "../tests/unit/error.rs"]
mod tests;
