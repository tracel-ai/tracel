use burn_central_client::schemas::RegisteredFunction;
pub use inventory;
use quote::ToTokens;

#[derive(Clone, Debug)]
pub struct FunctionMetadata {
    pub mod_path: &'static str,
    pub fn_name: &'static str,
    pub builder_fn_name: &'static str,
    pub routine_name: &'static str,
    pub proc_type: &'static str,
    pub token_stream: &'static [u8],
}

impl From<FunctionMetadata> for RegisteredFunction {
    fn from(val: FunctionMetadata) -> Self {
        let itemfn = syn_serde::json::from_slice::<syn::ItemFn>(val.token_stream)
            .expect("Should be able to parse token stream.");
        let syn_tree: syn::File =
            syn::parse2(itemfn.into_token_stream()).expect("Should be able to parse token stream.");
        let code_str = prettyplease::unparse(&syn_tree);
        RegisteredFunction {
            mod_path: val.mod_path.to_string(),
            fn_name: val.fn_name.to_string(),
            proc_type: val.proc_type.to_string(),
            code: code_str,
        }
    }
}

pub type LazyValue<T> = once_cell::sync::Lazy<T>;
pub struct Plugin<T: 'static>(pub &'static LazyValue<T>);

inventory::collect!(Plugin<FunctionMetadata>);

pub const fn make_static_lazy<T>(init: fn() -> T) -> LazyValue<T> {
    once_cell::sync::Lazy::new(init)
}

// macro that generates a flag with a given type and arbitrary parameters and submits it to the inventory
#[macro_export]
macro_rules! register_functions {
    ($type:ty, $init:expr) => {
        const _: () = {
            #[allow(non_upper_case_globals)]
            static FLAG: $crate::tools::functions_registry::LazyValue<$type> =
                $crate::tools::functions_registry::make_static_lazy(|| $init);

            $crate::tools::functions_registry::inventory::submit!(
                $crate::tools::functions_registry::Plugin(&FLAG)
            );
        };
    };
}

/// Need it for the macro to work
impl FunctionMetadata {
    pub fn new(
        mod_path: &'static str,
        fn_name: &'static str,
        builder_fn_name: &'static str,
        routine_name: &'static str,
        proc_type: &'static str,
        token_stream: &'static [u8],
    ) -> Self {
        Self {
            mod_path,
            fn_name,
            builder_fn_name,
            routine_name,
            proc_type,
            token_stream,
        }
    }
}

pub struct FunctionRegistry {
    functions: LazyValue<Vec<FunctionMetadata>>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self {
            functions: LazyValue::new(|| {
                inventory::iter::<Plugin<FunctionMetadata>>
                    .into_iter()
                    .map(|plugin| (*plugin.0).to_owned())
                    .collect()
            }),
        }
    }

    pub fn get_function_references(&self) -> &[FunctionMetadata] {
        &self.functions
    }

    pub fn get_registered_functions(&self) -> Vec<RegisteredFunction> {
        self.functions
            .iter()
            .map(|function| function.clone().into())
            .collect()
    }

    pub fn get_training_routine(&self) -> Vec<String> {
        self.functions
            .iter()
            .filter(|function| function.proc_type == "training")
            .map(|function| function.routine_name.to_string())
            .collect()
    }
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
