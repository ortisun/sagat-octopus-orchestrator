use sagat_octopus_orchestrator_sdk::{Client, ClientOptions, TokenSource};
fn main() {
    let client = Client::new(ClientOptions::new(
        "https://platform.example/v1",
        "payments",
        TokenSource::bearer("fixture-token"),
    ))
    .expect("a clean consumer can configure the SDK");
    assert!(!format!("{client:?}").contains("fixture-token"));
}
