use std::{path::PathBuf, sync::Arc};

use crate::{
    context::{ContextError, IntoProviders, Providers},
    inference::DefaultInferenceProvider,
};
#[derive(Debug, Clone)]
pub struct LocalBackend {
    pub(crate) path: PathBuf,
}

impl IntoProviders for LocalBackend {
    fn into_providers(self) -> Result<Providers, ContextError> {
        let backend = Arc::new(self);
        Ok(Providers {
            experiment: backend,
            inference: Arc::new(DefaultInferenceProvider::new()),
            model_registry: None,
            dataset: None,
        })
    }
}

impl LocalBackend {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}
