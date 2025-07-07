use crate::http::HttpClient;

///! This module provides the [BurnCentral] struct, which is used to interact with the Burn Central service.

/// This struct provides the main interface to interact with Burn Central.
pub struct BurnCentral {
    client: HttpClient,
}