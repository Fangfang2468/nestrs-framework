use nestrs_core::{
    __private::{
        ActivationError, ConstructionContext, ErasedService, FieldInjection, FieldInjectionTarget,
        Inject, InputPosition, InterfaceBinding, PrepareInput, StructComponent,
        prepare_bound_optional, prepare_bound_required, prepare_optional_absent, prepare_required,
    },
    lifetime::Lifetime,
    registration::{
        service_collection::ServiceCollection, service_identifier::ServiceIdentifier,
        service_source::ServiceSource, service_type::ServiceType,
    },
};

trait Mailer: Send + Sync {
    fn label(&self) -> &'static str;
    fn address(&self) -> usize;
}

trait MissingMailer: Send + Sync {}

struct SmtpMailer {
    label: &'static str,
}

impl Mailer for SmtpMailer {
    fn label(&self) -> &'static str {
        self.label
    }

    fn address(&self) -> usize {
        self as *const Self as usize
    }
}

struct Consumer {
    mailer: Inject<dyn Mailer>,
    missing: Option<Inject<dyn MissingMailer>>,
    concrete: Inject<SmtpMailer>,
}

fn construct_smtp_mailer(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(SmtpMailer { label: "smtp" }))
}

fn construct_consumer(mut context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    let mailer = context.take::<dyn Mailer>(InputPosition(0))?;
    let missing = context.take_optional::<dyn MissingMailer>(InputPosition(1))?;
    let concrete = context.take::<SmtpMailer>(InputPosition(2))?;
    Ok(ErasedService::new(Consumer {
        mailer,
        missing,
        concrete,
    }))
}

fn project_smtp_mailer(value: &SmtpMailer) -> &(dyn Mailer + 'static) {
    value
}

fn prepare_smtp_mailer_required(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<nestrs_core::__private::ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_required::<SmtpMailer, dyn Mailer>(context, position, input, project_smtp_mailer)
}

fn prepare_smtp_mailer_optional(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<nestrs_core::__private::ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_optional::<SmtpMailer, dyn Mailer>(context, position, input, project_smtp_mailer)
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECT_METADATA_INJECTABLE
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn smtp_mailer_metadata() -> StructComponent {
    StructComponent {
        service_identifier: ServiceIdentifier::from(ServiceType::create::<SmtpMailer>()),
        lifetime: Lifetime::Singleton,
        field_injections: vec![],
        constructor: construct_smtp_mailer,
        primary: false,
        source: ServiceSource::new("bound_injection.rs", 1, 1),
    }
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECT_METADATA_INJECTABLE
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn consumer_metadata() -> StructComponent {
    StructComponent {
        service_identifier: ServiceIdentifier::from(ServiceType::create::<Consumer>()),
        lifetime: Lifetime::Singleton,
        field_injections: vec![
            FieldInjection {
                field_index: 0,
                field_name: Some("mailer"),
                dependency_position: 0,
                service_identifier: ServiceIdentifier::from(ServiceType::create::<dyn Mailer>()),
                target: FieldInjectionTarget::TraitObject,
                component_definition: None,
                prepare_input: None,
                optional: false,
            },
            FieldInjection {
                field_index: 1,
                field_name: Some("missing"),
                dependency_position: 1,
                service_identifier: ServiceIdentifier::from(
                    ServiceType::create::<dyn MissingMailer>(),
                ),
                target: FieldInjectionTarget::TraitObject,
                component_definition: None,
                prepare_input: Some(prepare_optional_absent::<dyn MissingMailer> as PrepareInput),
                optional: true,
            },
            FieldInjection {
                field_index: 2,
                field_name: Some("concrete"),
                dependency_position: 2,
                service_identifier: ServiceIdentifier::from(ServiceType::create::<SmtpMailer>()),
                target: FieldInjectionTarget::Concrete,
                component_definition: None,
                prepare_input: Some(prepare_required::<SmtpMailer> as PrepareInput),
                optional: false,
            },
        ],
        constructor: construct_consumer,
        primary: false,
        source: ServiceSource::new("bound_injection.rs", 2, 1),
    }
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECT_METADATA_BIND
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn smtp_mailer_binding() -> InterfaceBinding {
    InterfaceBinding {
        service_type: ServiceType::create::<SmtpMailer>(),
        trait_type: ServiceType::create::<dyn Mailer>(),
        prepare_required: prepare_smtp_mailer_required,
        prepare_optional: prepare_smtp_mailer_optional,
    }
}

#[test]
fn trait_binding_projects_the_same_committed_concrete_arena_value() {
    let arena = ServiceCollection::new()
        .instantiate::<Consumer>()
        .expect("bound trait dependency should resolve through its concrete provider");
    let consumer = arena
        .get::<Consumer>()
        .expect("consumer should be committed to the arena");
    let mailer = arena
        .get::<SmtpMailer>()
        .expect("bound concrete provider should be committed exactly once");

    assert_eq!(consumer.mailer.label(), "smtp");
    assert_eq!(consumer.mailer.address(), consumer.concrete.address());
    assert_eq!(consumer.mailer.address(), mailer.address());
    assert!(consumer.missing.is_none());
}
