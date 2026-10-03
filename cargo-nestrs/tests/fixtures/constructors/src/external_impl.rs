//! impl 位于独立源文件，通过真实 Self 类型关联到主模块中的服务字段。
use constructor_library::Clock;
use nestrs::constructor;

impl super::ExternalService {
    #[constructor]
    fn create(clock: Clock) -> Self {
        Self { clock, value: 44 }
    }
}
