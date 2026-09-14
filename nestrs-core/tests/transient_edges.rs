use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

use nestrs_core::{
    __private::{
        ActivationError, ArenaServiceRef, BoundKeyPolicy, ClassProvider, ConstructionContext,
        Delivery, DependencyRequest, ErasedService, Injectable, InputPosition, Lifetime, Provider,
        ProviderCommon, ProviderSource, ServiceIdentifier, ServiceKey, ServiceSource, ServiceType,
        TraitBinding, prepare_bound_optional, prepare_bound_required, prepare_optional,
        prepare_required,
    },
    ServiceProvider,
};

type Inject<T> = nestrs_core::__private::Inject<T>;

trait KeyedTransientPort: Send + Sync {
    fn instance(&self) -> usize;
}

struct KeyedTransientAdapter {
    instance: usize,
}

impl KeyedTransientPort for KeyedTransientAdapter {
    fn instance(&self) -> usize {
        self.instance
    }
}

struct KeyedTransientRoot {
    first: Inject<dyn KeyedTransientPort>,
    second: Inject<dyn KeyedTransientPort>,
}

struct OptionalPresentTransient {
    instance: usize,
}

struct OptionalMissingTransient;

struct OptionalTransientRoot {
    first: Option<Inject<OptionalPresentTransient>>,
    second: Option<Inject<OptionalPresentTransient>>,
    missing: Option<Inject<OptionalMissingTransient>>,
}

struct FallbackGeneric<T> {
    instance: usize,
    _marker: PhantomData<T>,
}

struct FallbackGenericArgument;

struct FallbackGenericRoot {
    first: Inject<FallbackGeneric<FallbackGenericArgument>>,
    second: Inject<FallbackGeneric<FallbackGenericArgument>>,
}

struct ExplicitGeneric<T> {
    instance: usize,
    origin: &'static str,
    _marker: PhantomData<T>,
}

struct ExplicitGenericArgument;

struct ExplicitGenericRoot {
    first: Inject<ExplicitGeneric<ExplicitGenericArgument>>,
    second: Inject<ExplicitGeneric<ExplicitGenericArgument>>,
}

static KEYED_TRANSIENT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static OPTIONAL_TRANSIENT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static FALLBACK_GENERIC_MATERIALIZATIONS: AtomicUsize = AtomicUsize::new(0);
static FALLBACK_GENERIC_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static EXPLICIT_GENERIC_FALLBACK_MATERIALIZATIONS: AtomicUsize = AtomicUsize::new(0);
static EXPLICIT_GENERIC_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

fn identifier<T>() -> ServiceIdentifier
where
    T: Injectable + ?Sized,
{
    ServiceIdentifier::from(ServiceType::create::<T>())
}

fn keyed_identifier<T>(key: &'static str) -> ServiceIdentifier
where
    T: Injectable + ?Sized,
{
    ServiceIdentifier::new(Some(ServiceKey::Named(key)), ServiceType::create::<T>())
}

fn source(line: u32) -> ServiceSource {
    ServiceSource::new("tests/transient_edges.rs", line, 1)
}

fn common(lifetime: Lifetime, line: u32) -> ProviderCommon {
    ProviderCommon {
        lifetime,
        primary: false,
        source: source(line),
        cleanup: None,
    }
}

fn transient_common(line: u32) -> ProviderCommon {
    common(Lifetime::Transient, line)
}

fn singleton_common(line: u32) -> ProviderCommon {
    common(Lifetime::Singleton, line)
}

fn optional<T>(position: usize) -> DependencyRequest
where
    T: Injectable,
{
    DependencyRequest {
        declaration_position: position,
        input_position: InputPosition(position),
        token: identifier::<T>(),
        optional: true,
        label: None,
        delivery: Delivery::Direct(prepare_optional::<T>),
        provider_source: ProviderSource::Registered,
    }
}

fn materialized<T>(position: usize, callback: fn() -> Provider) -> DependencyRequest
where
    T: Injectable,
{
    DependencyRequest {
        declaration_position: position,
        input_position: InputPosition(position),
        token: identifier::<T>(),
        optional: false,
        label: None,
        delivery: Delivery::Direct(prepare_required::<T>),
        provider_source: ProviderSource::Materialize(callback),
    }
}

fn construct_keyed_transient_adapter(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(KeyedTransientAdapter {
        instance: KEYED_TRANSIENT_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_keyed_transient_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(KeyedTransientRoot {
        first: context.take::<dyn KeyedTransientPort>(InputPosition(0))?,
        second: context.take::<dyn KeyedTransientPort>(InputPosition(1))?,
    }))
}

fn construct_optional_present_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(OptionalPresentTransient {
        instance: OPTIONAL_TRANSIENT_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_optional_transient_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(OptionalTransientRoot {
        first: context.take_optional::<OptionalPresentTransient>(InputPosition(0))?,
        second: context.take_optional::<OptionalPresentTransient>(InputPosition(1))?,
        missing: context.take_optional::<OptionalMissingTransient>(InputPosition(2))?,
    }))
}

fn construct_fallback_generic<T>(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError>
where
    T: Injectable,
{
    Ok(ErasedService::new(FallbackGeneric::<T> {
        instance: FALLBACK_GENERIC_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
        _marker: PhantomData,
    }))
}

fn fallback_generic_provider<T>() -> Provider
where
    T: Injectable,
{
    Provider::Class(ClassProvider {
        provide: identifier::<FallbackGeneric<T>>(),
        common: transient_common(300),
        dependencies: Vec::new(),
        constructor: construct_fallback_generic::<T>,
    })
}

fn materialize_fallback_generic() -> Provider {
    FALLBACK_GENERIC_MATERIALIZATIONS.fetch_add(1, Ordering::SeqCst);
    fallback_generic_provider::<FallbackGenericArgument>()
}

fn construct_fallback_generic_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FallbackGenericRoot {
        first: context.take::<FallbackGeneric<FallbackGenericArgument>>(InputPosition(0))?,
        second: context.take::<FallbackGeneric<FallbackGenericArgument>>(InputPosition(1))?,
    }))
}

fn construct_explicit_generic<T>(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError>
where
    T: Injectable,
{
    Ok(ErasedService::new(ExplicitGeneric::<T> {
        instance: EXPLICIT_GENERIC_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
        origin: "explicit",
        _marker: PhantomData,
    }))
}

fn explicit_generic_provider<T>() -> Provider
where
    T: Injectable,
{
    Provider::Class(ClassProvider {
        provide: identifier::<ExplicitGeneric<T>>(),
        common: transient_common(400),
        dependencies: Vec::new(),
        constructor: construct_explicit_generic::<T>,
    })
}

fn materialize_explicit_generic() -> Provider {
    EXPLICIT_GENERIC_FALLBACK_MATERIALIZATIONS.fetch_add(1, Ordering::SeqCst);
    explicit_generic_provider::<ExplicitGenericArgument>()
}

fn construct_explicit_generic_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ExplicitGenericRoot {
        first: context.take::<ExplicitGeneric<ExplicitGenericArgument>>(InputPosition(0))?,
        second: context.take::<ExplicitGeneric<ExplicitGenericArgument>>(InputPosition(1))?,
    }))
}

fn project_keyed_transient_adapter(
    value: &KeyedTransientAdapter,
) -> &(dyn KeyedTransientPort + 'static) {
    value
}

fn prepare_keyed_transient_required(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_required::<KeyedTransientAdapter, dyn KeyedTransientPort>(
        context,
        position,
        input,
        project_keyed_transient_adapter,
    )
}

fn prepare_keyed_transient_optional(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_optional::<KeyedTransientAdapter, dyn KeyedTransientPort>(
        context,
        position,
        input,
        project_keyed_transient_adapter,
    )
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_transient_adapter_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: keyed_identifier::<KeyedTransientAdapter>("transient-port"),
        common: transient_common(100),
        dependencies: Vec::new(),
        constructor: construct_keyed_transient_adapter,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_transient_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<KeyedTransientRoot>(),
        common: singleton_common(110),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: keyed_identifier::<dyn KeyedTransientPort>("transient-port"),
                optional: false,
                label: Some("first keyed transient port"),
                delivery: Delivery::RequiresBinding,
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: keyed_identifier::<dyn KeyedTransientPort>("transient-port"),
                optional: false,
                label: Some("second keyed transient port"),
                delivery: Delivery::RequiresBinding,
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_keyed_transient_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn optional_present_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<OptionalPresentTransient>(),
        common: transient_common(200),
        dependencies: Vec::new(),
        constructor: construct_optional_present_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn optional_transient_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<OptionalTransientRoot>(),
        common: singleton_common(210),
        dependencies: vec![
            optional::<OptionalPresentTransient>(0),
            optional::<OptionalPresentTransient>(1),
            optional::<OptionalMissingTransient>(2),
        ],
        constructor: construct_optional_transient_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn fallback_generic_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FallbackGenericRoot>(),
        common: singleton_common(310),
        dependencies: vec![
            materialized::<FallbackGeneric<FallbackGenericArgument>>(
                0,
                materialize_fallback_generic,
            ),
            materialized::<FallbackGeneric<FallbackGenericArgument>>(
                1,
                materialize_fallback_generic,
            ),
        ],
        constructor: construct_fallback_generic_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn explicit_generic_provider_registration() -> Provider {
    explicit_generic_provider::<ExplicitGenericArgument>()
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn explicit_generic_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ExplicitGenericRoot>(),
        common: singleton_common(410),
        dependencies: vec![
            materialized::<ExplicitGeneric<ExplicitGenericArgument>>(
                0,
                materialize_explicit_generic,
            ),
            materialized::<ExplicitGeneric<ExplicitGenericArgument>>(
                1,
                materialize_explicit_generic,
            ),
        ],
        constructor: construct_explicit_generic_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(::nestrs_core::__private::REFLECTED_BINDINGS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_transient_port_binding() -> TraitBinding {
    TraitBinding {
        trait_type: ServiceType::create::<dyn KeyedTransientPort>(),
        concrete_type: ServiceType::create::<KeyedTransientAdapter>(),
        key_policy: BoundKeyPolicy::InheritRequestedKey,
        prepare_required: prepare_keyed_transient_required,
        prepare_optional: prepare_keyed_transient_optional,
        source: source(120),
    }
}

#[test]
fn keyed_trait_projection_creates_one_transient_occurrence_per_injection_edge() {
    KEYED_TRANSIENT_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let provider = ServiceProvider::<KeyedTransientRoot>::build()
        .expect("the keyed trait binding should resolve to its transient concrete provider");
    let root = provider.root();

    assert_eq!(root.first.instance(), 1);
    assert_eq!(root.second.instance(), 2);
    assert_eq!(KEYED_TRANSIENT_CONSTRUCTIONS.load(Ordering::SeqCst), 2);
}

#[test]
fn optional_transient_inputs_preserve_absence_and_do_not_share_present_occurrences() {
    OPTIONAL_TRANSIENT_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let provider = ServiceProvider::<OptionalTransientRoot>::build()
        .expect("optional transient inputs should not require a missing registration");
    let root = provider.root();

    assert_eq!(
        root.first.as_deref().map(|service| service.instance),
        Some(1)
    );
    assert_eq!(
        root.second.as_deref().map(|service| service.instance),
        Some(2)
    );
    assert!(root.missing.is_none());
    assert_eq!(OPTIONAL_TRANSIENT_CONSTRUCTIONS.load(Ordering::SeqCst), 2);
}

#[test]
fn closed_generic_fallback_caches_the_provider_blueprint_but_not_transient_occurrences() {
    FALLBACK_GENERIC_MATERIALIZATIONS.store(0, Ordering::SeqCst);
    FALLBACK_GENERIC_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let provider = ServiceProvider::<FallbackGenericRoot>::build()
        .expect("the closed generic fallback should materialize for a reachable transient");
    let root = provider.root();

    assert_eq!(root.first.instance, 1);
    assert_eq!(root.second.instance, 2);
    assert_eq!(FALLBACK_GENERIC_MATERIALIZATIONS.load(Ordering::SeqCst), 1);
    assert_eq!(FALLBACK_GENERIC_CONSTRUCTIONS.load(Ordering::SeqCst), 2);
}

#[test]
fn explicit_closed_generic_candidate_wins_over_the_transient_fallback() {
    EXPLICIT_GENERIC_FALLBACK_MATERIALIZATIONS.store(0, Ordering::SeqCst);
    EXPLICIT_GENERIC_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let provider = ServiceProvider::<ExplicitGenericRoot>::build()
        .expect("an explicit closed generic provider should beat its materialization callback");
    let root = provider.root();

    assert_eq!(root.first.origin, "explicit");
    assert_eq!(root.second.origin, "explicit");
    assert_eq!(root.first.instance, 1);
    assert_eq!(root.second.instance, 2);
    assert_eq!(
        EXPLICIT_GENERIC_FALLBACK_MATERIALIZATIONS.load(Ordering::SeqCst),
        0
    );
    assert_eq!(EXPLICIT_GENERIC_CONSTRUCTIONS.load(Ordering::SeqCst), 2);
}
