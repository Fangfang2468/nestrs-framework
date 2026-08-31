use nestrs_injection::registration::{
    service_identifier::ServiceIdentifier, service_type::ServiceType,
};

#[test]
fn create_service_identifier_by_type() {
    struct Test;

    let service_type = ServiceType::create::<Test>();

    let service_identifier = ServiceIdentifier::from(service_type);

    println!("{service_identifier:#?}");
    // println!("{service_identifier}");
}


#[test]
fn create_service_identifier_by_generic_type() {
    trait Repository<T>: Send + Sync + 'static {
        
    }

    struct User;
    struct Post;

    let service_type1 = ServiceType::create::<dyn Repository<User>>();
    let service_identifier1 = ServiceIdentifier::from(service_type1);

    println!("{service_identifier1:#?}");


    let service_type2 = ServiceType::create::<dyn Repository<Post>>();
    let service_identifier2 = ServiceIdentifier::from(service_type2);
    println!("{service_identifier2:#?}");

    assert_ne!(service_identifier1, service_identifier2, "service_identifier1 与 service_identifier2 服务相同");
}