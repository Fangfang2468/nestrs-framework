//! 捕获 panic 后仍需回收其用户载荷；这一步也是用户代码的执行边界。

use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
};

/// 持有已捕获的 panic，避免其析构再次展开协调器或释放队列的栈。
pub(crate) struct PanicPayload(Option<Box<dyn Any + Send>>);

impl PanicPayload {
    /// 接管原始 panic 载荷，使其后续析构也经过保护边界。
    pub(crate) fn new(payload: Box<dyn Any + Send>) -> Self {
        Self(Some(payload))
    }

    /// 提取可读详情，并在回收载荷时保留可能的二次 panic 信息。
    pub(crate) fn into_message(self) -> String {
        let message = Self::message(self.0.as_deref().expect("panic 载荷尚未消费")).to_owned();
        self.finish_message(message)
    }

    /// 保留调用方已有的诊断（如 Tokio 的任务编号），并在回收失败时补充原因。
    pub(crate) fn finish_message(mut self, mut message: String) -> String {
        if let Some(detail) = self.dispose() {
            message.push_str("；panic 载荷 Drop 再次 panic：");
            message.push_str(&detail);
        }
        message
    }

    /// 普通同步 Drop 仍可把原始 panic 交还给调用者，而不是改成字符串 panic。
    pub(crate) fn resume(mut self) -> ! {
        resume_unwind(self.0.take().expect("panic 载荷尚未消费"))
    }

    /// 有界回收用户载荷；二次析构 panic 不再递归触发回收。
    fn dispose(&mut self) -> Option<String> {
        let payload = self.0.take()?;
        let secondary = catch_unwind(AssertUnwindSafe(|| drop(payload))).err()?;
        let detail = Self::message(secondary.as_ref()).to_owned();

        // String / &str 没有用户析构，可正常回收。任意自定义二级载荷的 Drop
        // 可能继续产生同类 panic；仅放弃这个二级载荷，保证处理有界且不递归。
        // 原始载荷已经尝试析构，不会因为其类型未知而直接泄漏。
        if secondary.is::<String>() || secondary.is::<&'static str>() {
            drop(secondary);
        } else {
            std::mem::forget(secondary);
        }
        Some(detail)
    }

    /// 读取字符串载荷，其他类型只返回统一占位说明。
    fn message(payload: &(dyn Any + Send)) -> &str {
        payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("未提供字符串 panic 信息")
    }
}

impl Drop for PanicPayload {
    /// 在未显式消费载荷时执行同样受保护的回收流程。
    fn drop(&mut self) {
        let _ = self.dispose();
    }
}

#[cfg(test)]
#[path = "../tests/unit/panic_payload.rs"]
mod tests;
