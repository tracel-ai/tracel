use crate::tools::function_discovery::FunctionMetadata;

pub struct FunctionRegistry {
    functions: Vec<FunctionMetadata>,
}

impl FunctionRegistry {
    pub fn new(functions: Vec<FunctionMetadata>) -> Self {
        Self { functions }
    }

    pub fn get_function_references(&self) -> &[FunctionMetadata] {
        &self.functions
    }

    // pub fn get_registered_functions(&self) -> Vec<RegisteredFunctionRequest> {
    //     self.functions
    //         .iter()
    //         .map(|function| function.clone().into())
    //         .collect()
    // }

    pub fn get_training_routine(&self) -> Vec<String> {
        self.functions
            .iter()
            .filter(|function| function.proc_type == "training")
            .map(|function| function.routine_name.clone())
            .collect()
    }
}
