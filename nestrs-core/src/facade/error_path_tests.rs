use crate::service::{ServiceIdentifier, ServiceSource, ServiceType};

use super::ResolveError;

#[test]
fn twenty_thousand_failure_frames_format_and_drop_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            struct FailingService;
            let identifier = ServiceIdentifier::from(ServiceType::create::<FailingService>());
            let mut error = ResolveError::new("original factory failure".to_owned());
            for line in 1..=20_000 {
                error = ResolveError::dependency(
                    &identifier,
                    ServiceSource::new("deep_failure.rs", line, 1),
                    error,
                );
            }
            let rendered = error.to_string();
            assert!(rendered.starts_with("服务解析失败: original factory failure\n"));
            assert_eq!(rendered.lines().count(), 20_001);
            assert!(
                rendered
                    .lines()
                    .nth(1)
                    .unwrap()
                    .ends_with("deep_failure.rs:20000:1)")
            );
            assert!(
                rendered
                    .lines()
                    .last()
                    .unwrap()
                    .ends_with("deep_failure.rs:1:1)")
            );
            drop(error);
        })
        .unwrap()
        .join()
        .unwrap();
}
