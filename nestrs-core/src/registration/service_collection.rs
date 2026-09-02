use crate::{__private::REFLECT_METADATA_BIND, registration::service_descriptor::ServiceDescriptor};


/// 注册服务集合中，存储了所有注册的服务描述符。
pub struct ServiceCollection {
    pub(crate) services: Vec<ServiceDescriptor>,
}


impl ServiceCollection {
    pub fn new() -> Self {

        // bind 元数据
        let bind_metadata = REFLECT_METADATA_BIND
            .iter()
            .map(|f| f())
            .collect::<Vec<_>>();

        println!("----------------------------------- #[bind] 元数据 -----------------------------------");
        println!("{bind_metadata:#?}");

        Self {
            services: vec![]
        }
    }
}