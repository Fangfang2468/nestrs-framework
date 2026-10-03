use std::{
    panic::{AssertUnwindSafe, catch_unwind, panic_any},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::PanicPayload;

#[test]
fn string_messages_are_preserved_and_unknown_payloads_are_released() {
    assert_eq!(
        PanicPayload::new(Box::new("literal")).into_message(),
        "literal"
    );
    assert_eq!(
        PanicPayload::new(Box::new(String::from("owned"))).into_message(),
        "owned"
    );
    struct Counted(Arc<AtomicUsize>);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let message = PanicPayload::new(Box::new(Counted(drops.clone()))).into_message();
    assert_eq!(message, "未提供字符串 panic 信息");
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn secondary_string_panic_is_reported_without_escaping() {
    struct Panics;
    impl Drop for Panics {
        fn drop(&mut self) {
            panic!("payload destructor sentinel");
        }
    }
    let message = PanicPayload::new(Box::new(Panics)).finish_message("original".into());
    assert_eq!(
        message,
        "original；panic 载荷 Drop 再次 panic：payload destructor sentinel"
    );
    // 包装值即使未经显式转换就被丢弃，也不能把用户 panic 传出。
    drop(PanicPayload::new(Box::new(Panics)));
}

#[test]
fn self_reproducing_payload_is_bounded_to_one_destructor_call() {
    struct Repeats(Arc<AtomicUsize>);
    impl Drop for Repeats {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic_any(Repeats(self.0.clone()));
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let message = PanicPayload::new(Box::new(Repeats(drops.clone()))).into_message();
    assert!(message.contains("panic 载荷 Drop 再次 panic"));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn resume_preserves_original_payload_identity() {
    let original = Box::new(123_u32);
    let pointer = original.as_ref() as *const u32;
    let payload = catch_unwind(AssertUnwindSafe(|| PanicPayload::new(original).resume()))
        .expect_err("resume 应把原始 panic 交给调用者");
    let recovered = payload.downcast::<u32>().unwrap();
    assert_eq!(recovered.as_ref() as *const u32, pointer);
}
