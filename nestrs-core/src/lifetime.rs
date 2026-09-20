#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceLifetime {
    Singleton,

    Scoped,

    Transient,
}
