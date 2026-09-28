#[nestrs::injectable]
pub struct Exported {
    #[nestrs::value(42)]
    pub number: u32,
}

pub fn answer() -> u32 {
    42
}
