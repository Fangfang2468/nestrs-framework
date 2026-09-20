#![forbid(unsafe_code)]

use nestrs_core::__private::Injection;
use nestrs_macro::injectable;

trait Database: Send + Sync {
    fn query(&self);
}

trait Audit: Send + Sync {
    fn record(&self);
}

struct Indexed;

impl Indexed {
    fn find(&self) {}
}

#[injectable]
struct Consumer {
    #[inject]
    database: dyn Database,
    #[inject(key = "audit")]
    audit: Option<dyn Audit>,
    #[inject(7)]
    indexed: Indexed,
}

fn accepts_required<T: ?Sized>(_: Injection<T>) {}

fn accepts_optional<T: ?Sized>(_: Option<Injection<T>>) {}

fn checks_rewritten_field_types(consumer: Consumer) {
    let Consumer {
        database,
        audit,
        indexed,
    } = consumer;

    accepts_required(database);
    accepts_optional(audit);
    accepts_required(indexed);
}

impl Consumer {
    fn checks_read_only_deref(&self) {
        self.database.query();
        if let Some(audit) = &self.audit {
            audit.record();
        }
        self.indexed.find();
    }
}

fn main() {}
