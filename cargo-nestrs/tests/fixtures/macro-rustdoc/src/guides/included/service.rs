#[nestrs::injectable]
pub struct IncludedService;

pub trait IncludedContract: Send + Sync {
    fn answer(&self) -> usize;
}

impl IncludedContract for IncludedService {
    fn answer(&self) -> usize {
        42
    }
}
