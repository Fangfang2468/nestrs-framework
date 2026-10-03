//! Ordinary Rust code: core-control changes only the manifest dependency graph.
pub trait Run<P> {
    fn run(&self, provider: &P);
}

pub fn invoke<T: Run<P>, P>(runner: &T, provider: &P) {
    runner.run(provider);
}

pub fn erase<T: Run<P> + 'static, P>(runner: T) -> Box<dyn Run<P>> {
    Box::new(runner)
}

pub fn invoke_closure<T: Run<P>, P>(runner: &T, provider: &P) {
    let call = || runner.run(provider);
    call();
}

pub trait DefaultRun<P>: Run<P> {
    fn run_default(&self, provider: &P) {
        self.run(provider);
    }
}
impl<T: Run<P>, P> DefaultRun<P> for T {}

pub fn invoke_default<T: DefaultRun<P>, P>(runner: &T, provider: &P) {
    runner.run_default(provider);
}

pub trait AssociatedRun<P>: Run<P> + Sized {
    const CALL: fn(&Self, &P) = Self::run;
}
impl<T: Run<P>, P> AssociatedRun<P> for T {}

pub fn invoke_associated<T: AssociatedRun<P>, P>(runner: &T, provider: &P) {
    (T::CALL)(runner, provider);
}

pub fn invoke_unexecuted<T: Run<P>, P>(runner: &T, provider: &P) {
    if false {
        runner.run(provider);
    }
}

pub fn invoke_unexecuted_erasure<T: Run<P> + 'static, P>(runner: T, provider: &P) {
    if false {
        erase(runner).run(provider);
    }
}

pub fn invoke_unexecuted_associated<T: AssociatedRun<P>, P>(runner: &T, provider: &P) {
    if false {
        (T::CALL)(runner, provider);
    }
}

// Family is the only bound on these entry points. Its query-relevant Run bound
// belongs to an associated type, and may be absent from the function signature.
pub trait Family<P> {
    type Target: Run<P> + Default + 'static;
}
pub trait NestedFamily<P> {
    type Inner: Family<P>;
}
pub fn invoke_family<M: Family<P>, P>(runner: &M::Target, provider: &P) {
    runner.run(provider);
}
pub fn invoke_family_default<M: Family<P>, P>(provider: &P) {
    M::Target::default().run(provider);
}
pub fn invoke_family_return<M: Family<P>, P>(provider: &P) -> M::Target {
    let runner = M::Target::default();
    runner.run(provider);
    runner
}
pub fn invoke_family_nested<M: NestedFamily<P>, P>(provider: &P) {
    <M::Inner as Family<P>>::Target::default().run(provider);
}
pub fn invoke_family_dynamic<M: Family<P>, P>(provider: &P) {
    erase(M::Target::default()).run(provider);
}
