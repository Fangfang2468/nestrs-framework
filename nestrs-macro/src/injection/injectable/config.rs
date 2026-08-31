use crate::injection::attrs::{lifetime::ServiceLifetime, service_key::ServiceKey};

use zyn::{Attribute, syn::Path};



// #[zyn("injectable")] 表示解析 #[injectable(...)] 的参数
// #[zyn(default)] / #[zyn(default = "...")] 允许参数缺省
#[derive(Attribute, Clone, Debug)]
#[zyn("injectable")]
pub struct InjectableConfig {
    /// 服务生命周期；未声明时使用单例。
    #[zyn(default = ServiceLifetime::Singleton)]
    pub lifetime: ServiceLifetime,

    /// 服务输出使用的静态限定符。
    #[zyn(default = ServiceKey::Nil)]
    pub key: ServiceKey,

    /// 输出服务的异步 cleanup 回调路径。
    #[zyn(default)]
    pub cleanup: Option<Path>,
}
