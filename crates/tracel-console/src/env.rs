use tracel_client::console::Env;

const TRACEL_ENV: &str = "TRACEL_ENV";

pub fn from_environment() -> Env {
    let Ok(value) = std::env::var(TRACEL_ENV) else {
        return Env::Production;
    };

    match value.as_str() {
        "Development" => Env::Development,
        other => other
            .strip_prefix("Staging(")
            .and_then(|rest| rest.strip_suffix(')'))
            .and_then(|number| number.parse().ok())
            .map(Env::Staging)
            .unwrap_or(Env::Production),
    }
}
