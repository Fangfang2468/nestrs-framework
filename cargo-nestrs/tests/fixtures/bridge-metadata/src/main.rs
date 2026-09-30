fn main() {
    assert_eq!(nestrs_bridge_consumer::answer(), 42);
    assert_eq!(nestrs_bridge_consumer::resolve_number(), 42);
    assert_eq!(
        std::mem::size_of::<nestrs_bridge_consumer::Exported>(),
        std::mem::size_of::<u32>(),
    );
    println!("downstream metadata without a core dependency passed");
}
