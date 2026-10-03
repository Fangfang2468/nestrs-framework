#![allow(dead_code)]

// 私有模块的完整类型路径使 cause 超过终端阈值，用户正文仍只显示短服务名。
mod source_location_and_full_dependency_path_regression {
    use nestrs::injectable;

    macro_rules! services {
        ($($service:ident => $dependency:ident),* $(,)?) => {$(
            #[injectable]
            struct $service {
                #[inject]
                dependency: $dependency,
            }
        )*};
    }

    services!(
        LongCycleService01 => LongCycleService02,
        LongCycleService02 => LongCycleService03,
        LongCycleService03 => LongCycleService04,
        LongCycleService04 => LongCycleService05,
        LongCycleService05 => LongCycleService06,
        LongCycleService06 => LongCycleService07,
        LongCycleService07 => LongCycleService08,
        LongCycleService08 => LongCycleService09,
        LongCycleService09 => LongCycleService10,
        LongCycleService10 => LongCycleService11,
        LongCycleService11 => LongCycleService12,
        LongCycleService12 => LongCycleService01,
    );
}

fn main() {}
