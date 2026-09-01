#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lifetime {
    
    Singleton,
    
    Scoped,

    Transient,
}