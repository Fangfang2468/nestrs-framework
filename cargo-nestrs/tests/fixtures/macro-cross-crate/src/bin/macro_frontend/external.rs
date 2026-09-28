use nestrs::injectable;
#[injectable]
pub struct External {
    #[inject]
    port: dyn crate::Port,
}
impl External {
    pub fn number(&self) -> u32 {
        self.port.number()
    }
}
