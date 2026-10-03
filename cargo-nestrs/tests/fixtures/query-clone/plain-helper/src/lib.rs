//! Ordinary upstream helpers: no Nestrs dependency or query summaries.
pub fn tuple<T: Clone>(value: T) {
    drop((value,).clone());
}

pub fn nested<T: Clone>(value: T) {
    drop((0u8, (value,)).clone());
}

pub fn closure<T: Clone>(value: T) {
    let captured = move || drop(value);
    drop(captured.clone());
}

pub fn unexecuted<T: Clone>(value: T) {
    if false {
        drop((value,).clone());
    }
}
