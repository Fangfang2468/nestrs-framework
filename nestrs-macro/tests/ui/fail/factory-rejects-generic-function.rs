use nestrs_macro::factory;

#[factory]
fn generic_factory<T>() -> u8 {
    let _ = std::marker::PhantomData::<T>;
    1
}

fn main() {}
