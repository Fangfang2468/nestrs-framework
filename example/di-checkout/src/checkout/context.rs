use nestrs::injectable;

/// 请求级上下文在同一个 scope 内复用；不同用户请求应创建不同 scope。
#[injectable(lifetime = Scoped, cleanup = "cleanup_request_context")]
pub(crate) struct RequestContext {
    #[value(crate::observe::created("RequestContext"))]
    id: usize,
}

impl RequestContext {
    pub(crate) fn id(&self) -> usize {
        self.id
    }
}

async fn cleanup_request_context() {
    crate::observe::event("cleanup hook RequestContext（零参数，仅记录钩子调用）");
}

impl Drop for RequestContext {
    fn drop(&mut self) {
        crate::observe::event(format!("drop RequestContext #{}", self.id));
    }
}
