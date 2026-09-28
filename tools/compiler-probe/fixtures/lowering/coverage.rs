#![allow(dead_code)]

mod external;

#[derive(Debug)]
#[nestrs::injectable]
struct Root {
    value: String,
}

type LocalName = String;

macro_rules! generated {
    ($ty:ty) => {
        #[nestrs::injectable]
        #[derive(Debug)]
        struct Generated {
            value: $ty,
        }
    };
}

generated!(LocalName);

#[cfg(feature = "alternate")]
#[nestrs::injectable]
struct Alternate;

#[cfg(not(feature = "alternate"))]
#[nestrs::injectable]
struct DefaultBranch;

#[cfg(any())]
#[nestrs::injectable]
struct Disabled;

fn main() {
    let generated = Generated {
        value: "macro call-site type resolves".to_owned(),
    };
    assert!(format!("{generated:?}").contains("macro call-site type resolves"));
    external::check();
}
