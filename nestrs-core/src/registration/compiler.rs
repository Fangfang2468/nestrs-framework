//! Type-bearing markers consumed by the versioned compiler adapter.
//!
//! These calls are emitted only in registration descriptions. They carry no
//! runtime state and do not register, construct, or resolve a service. They must
//! remain calls in encoded MIR so downstream closed generic blueprints can be
//! analyzed before monomorphization; LLVM can still eliminate their empty bodies.
//! Keeping
//! separate identities lets the compiler distinguish providers, requests and
//! explicit bindings after macro expansion and type checking.

/// Static key payload recognized by the versioned compiler adapter.
///
/// Keeping the declaration key beside its type lets discovery prefer an
/// explicit provider over a generic blueprint for the exact service identity.
/// It does not allocate a runtime `ServiceKey` or add registration state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompilerKey {
    Default,
    Named(&'static str),
    Indexed(usize),
}

#[inline(never)]
pub const fn compiler_provider<T: ?Sized>(_key: CompilerKey) {
    let _ = core::marker::PhantomData::<T>;
}

#[inline(never)]
pub const fn compiler_request<T: ?Sized>() {
    let _ = core::marker::PhantomData::<T>;
}

#[inline(never)]
pub const fn compiler_binding<C: ?Sized, I: ?Sized>() {
    let _ = (
        core::marker::PhantomData::<C>,
        core::marker::PhantomData::<I>,
    );
}
