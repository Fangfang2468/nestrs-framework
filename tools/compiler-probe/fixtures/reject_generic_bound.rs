#![allow(dead_code)]

use std::marker::PhantomData;

trait Store {}
trait Entity {}

struct Repository<T>(PhantomData<T>);
struct NotAnEntity;

impl<T: Entity> Store for Repository<T> {}

fn invalid_projection(value: &Repository<NotAnEntity>) -> &dyn Store {
    value
}
