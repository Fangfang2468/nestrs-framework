#[nestrs::injectable]
pub struct External {
    #[nestrs::value(23)]
    pub number: usize,
}
