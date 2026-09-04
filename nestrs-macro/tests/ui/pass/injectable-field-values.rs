use nestrs_macro::injectable;

const BASE_RETRIES: u64 = 2;
static STATIC_RETRY_INCREMENT: u64 = 1;
const DEFAULT_NAME: &str = "default-name";
static STATIC_LABEL: &str = "static-label";

fn prefixed_name(prefix: &str) -> String {
    format!("{prefix}-controller")
}

#[injectable]
struct Values {
    #[value("123")]
    name: String,
    #[value(DEFAULT_NAME)]
    default_name: String,
    #[value(STATIC_LABEL)]
    static_label: String,
    #[value(3)]
    timestamp: u64,
    #[value(BASE_RETRIES + STATIC_RETRY_INCREMENT)]
    retries: u64,
    #[value(prefixed_name("user"))]
    label: String,
    #[value({
        let seed = 40;
        seed + 2
    })]
    answer: usize,
}

#[injectable]
struct TupleValues(#[value(1 + 2)] usize, #[value("ok")] &'static str);

#[injectable]
struct UnitValues;

fn main() {}
