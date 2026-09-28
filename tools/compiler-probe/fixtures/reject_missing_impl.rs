#![allow(dead_code)]

trait Store {}

struct MissingStore;

fn invalid_projection(value: &MissingStore) -> &dyn Store {
    value
}
