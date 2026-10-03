//! 具名生成项必须与业务同名常量和类型隔离，同时保留原始业务名称解析。
#![allow(non_upper_case_globals, non_camel_case_types)]

use nestrs_core::ServiceProvider;

mod factory_items {
    use nestrs::factory;

    pub const __nestrs_factory_provider_for_first: usize = 11;
    pub const __nestrs_factory_provider_for_second: usize = 17;
    pub const __nestrs_factory_provider_for_type: usize = 23;

    pub struct First(pub usize);
    pub struct Second(pub usize);
    pub struct Raw(pub usize);

    #[factory]
    fn first() -> First {
        First(__nestrs_factory_provider_for_first)
    }

    #[factory]
    async fn second(first: First) -> Second {
        Second(first.0 + __nestrs_factory_provider_for_second)
    }

    #[cfg_attr(all(), factory)]
    fn r#type() -> Raw {
        Raw(__nestrs_factory_provider_for_type)
    }
}

mod carrier_items {
    use nestrs::injectable;

    pub struct __NestrsConditionalFieldsForFirst {
        pub value: usize,
    }
    pub struct __NestrsConditionalFieldsForSecond(pub usize);
    pub struct __NestrsConditionalFieldsFortype(pub usize);

    #[injectable]
    pub struct First {
        #[cfg(any())]
        disabled: MissingFirst,
        #[value(__NestrsConditionalFieldsForFirst { value: 31 })]
        pub business: __NestrsConditionalFieldsForFirst,
    }

    #[injectable]
    pub struct Second {
        #[cfg_attr(all(), cfg(any()))]
        disabled: MissingSecond,
        #[cfg_attr(all(), value(__NestrsConditionalFieldsForSecond(37)))]
        pub business: __NestrsConditionalFieldsForSecond,
    }

    #[cfg_attr(all(), injectable)]
    pub struct r#type {
        #[cfg(any())]
        disabled: MissingRaw,
        #[cfg_attr(all(), value(__NestrsConditionalFieldsFortype(41)))]
        pub business: __NestrsConditionalFieldsFortype,
    }
}

mod macro_items {
    // 同一宏展开出的两个 provider/carrier 以及调用方传入的 raw ident 仍须各自隔离。
    macro_rules! declare {
        ($factory:ident, $constant:ident, $product:ident, $service:ident, $carrier:ident, $value:expr) => {
            pub const $constant: usize = $value;
            pub struct $product(pub usize);
            pub struct $carrier(pub usize);

            #[cfg_attr(all(), nestrs::factory)]
            fn $factory() -> $product {
                $product($constant)
            }

            #[cfg_attr(all(), nestrs::injectable)]
            pub struct $service {
                #[cfg_attr(all(), cfg(any()))]
                disabled: MissingMacroField,
                #[cfg_attr(all(), nestrs::inject)]
                pub product: $product,
                #[cfg_attr(all(), value($carrier($constant + 1)))]
                pub business: $carrier,
            }
        };
    }

    declare!(
        make,
        __nestrs_factory_provider_for_make,
        Product,
        Service,
        __NestrsConditionalFieldsForService,
        43
    );
    declare!(
        r#match,
        __nestrs_factory_provider_for_match,
        RawProduct,
        r#type,
        __NestrsConditionalFieldsFortype,
        47
    );
}

#[tokio::test(flavor = "current_thread")]
async fn factory_registration_items_preserve_business_constants() {
    let provider = ServiceProvider::build(None).await.unwrap();
    let first = provider
        .get_required_service::<factory_items::First>()
        .await
        .unwrap();
    let second = provider
        .get_required_service::<factory_items::Second>()
        .await
        .unwrap();
    let raw = provider
        .get_required_service::<factory_items::Raw>()
        .await
        .unwrap();

    assert_eq!(factory_items::__nestrs_factory_provider_for_first, 11);
    assert_eq!(factory_items::__nestrs_factory_provider_for_second, 17);
    assert_eq!(factory_items::__nestrs_factory_provider_for_type, 23);
    assert_eq!(first.0, 11);
    assert_eq!(second.0, 28);
    assert_eq!(raw.0, 23);
    provider.dispose_async().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn conditional_carriers_preserve_business_type_identity() {
    let provider = ServiceProvider::build(None).await.unwrap();
    let first = provider
        .get_required_service::<carrier_items::First>()
        .await
        .unwrap();
    let second = provider
        .get_required_service::<carrier_items::Second>()
        .await
        .unwrap();
    let raw = provider
        .get_required_service::<carrier_items::r#type>()
        .await
        .unwrap();

    let business_first: &carrier_items::__NestrsConditionalFieldsForFirst = &first.business;
    let business_second: &carrier_items::__NestrsConditionalFieldsForSecond = &second.business;
    let business_raw: &carrier_items::__NestrsConditionalFieldsFortype = &raw.business;
    assert_eq!(business_first.value, 31);
    assert_eq!(business_second.0, 37);
    assert_eq!(business_raw.0, 41);
    assert_eq!(
        carrier_items::__NestrsConditionalFieldsForFirst { value: 53 }.value,
        53
    );
    assert_eq!(carrier_items::__NestrsConditionalFieldsForSecond(59).0, 59);
    assert_eq!(carrier_items::__NestrsConditionalFieldsFortype(61).0, 61);
    provider.dispose_async().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn repeated_macro_expansions_preserve_cfg_and_raw_business_names() {
    let provider = ServiceProvider::build(None).await.unwrap();
    let service = provider
        .get_required_service::<macro_items::Service>()
        .await
        .unwrap();
    let raw = provider
        .get_required_service::<macro_items::r#type>()
        .await
        .unwrap();

    let business: &macro_items::__NestrsConditionalFieldsForService = &service.business;
    let raw_business: &macro_items::__NestrsConditionalFieldsFortype = &raw.business;
    assert_eq!(macro_items::__nestrs_factory_provider_for_make, 43);
    assert_eq!(macro_items::__nestrs_factory_provider_for_match, 47);
    assert_eq!(service.product.0, 43);
    assert_eq!(business.0, 44);
    assert_eq!(raw.product.0, 47);
    assert_eq!(raw_business.0, 48);
    assert_eq!(macro_items::__NestrsConditionalFieldsForService(67).0, 67);
    assert_eq!(macro_items::__NestrsConditionalFieldsFortype(71).0, 71);
    provider.dispose_async().await.unwrap();
}
