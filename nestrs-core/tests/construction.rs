use nestrs_core::{
    arena::Arena,
    registration::{service_identifier::ServiceIdentifier, service_type::ServiceType},
};

#[test]
fn arena_reads_a_committed_public_root_without_exposing_its_injection_pointer() {
    let identifier = ServiceIdentifier::from(ServiceType::create::<String>());
    let mut arena = Arena::new();
    arena
        .insert(identifier, String::from("root service"))
        .expect("concrete root should commit to the arena");

    assert_eq!(
        arena
            .get::<String>()
            .expect("committed root should be readable"),
        "root service"
    );
}
