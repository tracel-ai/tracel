use burn_central_client::StationClient;

#[derive(Debug, Clone)]
pub struct StationBackend {
    pub client: StationClient,
}

impl StationBackend {
    fn new(client: StationClient) -> Self {
        Self { client }
    }
}
