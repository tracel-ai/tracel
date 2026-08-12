use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracel_models::{ModelsError, VersionId};

#[derive(Clone, Default)]
pub(crate) struct ModelVersionRoutes {
    routes: Arc<Mutex<HashMap<(String, VersionId), u32>>>,
}

impl ModelVersionRoutes {
    pub(crate) fn get(&self, model: &str, id: &VersionId) -> Result<Option<u32>, ModelsError> {
        self.routes
            .lock()
            .map_err(|_| ModelsError::InvalidResponse("model version route state failed".into()))
            .map(|routes| routes.get(&(model.to_string(), id.clone())).copied())
    }

    pub(crate) fn remember(
        &self,
        model: &str,
        id: VersionId,
        version: u32,
    ) -> Result<(), ModelsError> {
        self.routes
            .lock()
            .map_err(|_| ModelsError::InvalidResponse("model version route state failed".into()))?
            .insert((model.to_string(), id), version);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_private_to_their_model_and_shared_across_clones() {
        let routes = ModelVersionRoutes::default();
        routes
            .remember("resnet", VersionId::new("opaque-id"), 42)
            .unwrap();

        assert_eq!(
            routes
                .clone()
                .get("resnet", &VersionId::new("opaque-id"))
                .unwrap(),
            Some(42)
        );
        assert_eq!(
            routes.get("other", &VersionId::new("opaque-id")).unwrap(),
            None
        );
    }
}
