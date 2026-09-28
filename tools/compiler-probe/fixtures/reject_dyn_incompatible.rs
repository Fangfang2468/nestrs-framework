#![allow(dead_code)]

trait Store {
    fn generic_method<T>(&self, value: T);
}

struct ConcreteStore;

impl Store for ConcreteStore {
    fn generic_method<T>(&self, _value: T) {}
}

fn invalid_projection(value: &ConcreteStore) -> &dyn Store {
    value
}
