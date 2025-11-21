use burn_central_client::request::RegisteredFunctionRequest;
use quote::ToTokens;

use crate::tools::function_discovery::FunctionMetadata;

impl From<FunctionMetadata> for RegisteredFunctionRequest {
    fn from(val: FunctionMetadata) -> Self {
        let code_str = if val.token_stream.is_empty() {
            // If no token stream is available, create a placeholder function
            format!(
                "fn {}() {{\n    // Function implementation not available\n}}",
                val.fn_name
            )
        } else {
            match syn_serde::json::from_slice::<syn::ItemFn>(&val.token_stream) {
                Ok(itemfn) => match syn::parse2(itemfn.into_token_stream()) {
                    Ok(syn_tree) => prettyplease::unparse(&syn_tree),
                    Err(_) => format!(
                        "fn {}() {{\n    // Failed to parse token stream\n}}",
                        val.fn_name
                    ),
                },
                Err(_) => format!(
                    "fn {}() {{\n    // Failed to deserialize token stream\n}}",
                    val.fn_name
                ),
            }
        };

        RegisteredFunctionRequest {
            mod_path: val.mod_path,
            fn_name: val.fn_name,
            proc_type: val.proc_type,
            code: code_str,
            routine: val.routine_name,
        }
    }
}

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

    pub fn get_registered_functions(&self) -> Vec<RegisteredFunctionRequest> {
        self.functions
            .iter()
            .map(|function| function.clone().into())
            .collect()
    }

    pub fn get_training_routine(&self) -> Vec<String> {
        self.functions
            .iter()
            .filter(|function| function.proc_type == "training")
            .map(|function| function.routine_name.clone())
            .collect()
    }
}
