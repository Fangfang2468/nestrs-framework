#![allow(dead_code)]

use nestrs::injectable;

struct Missing;

macro_rules! declare_service {
    ($service:ident, $dependency:ty) => {
        #[injectable]
        struct $service {
            #[inject]
            dependency: $dependency,
        }
    };
}

declare_service!(Application, Missing);

fn main() {}
