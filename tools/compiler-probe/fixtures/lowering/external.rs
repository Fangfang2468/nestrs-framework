#[nestrs::injectable]
#[derive(Debug)]
struct External {
    value: String,
}

pub fn check() {
    let value = External {
        value: "external private declaration".to_owned(),
    };
    assert_eq!(value.value, "external private declaration");
}
