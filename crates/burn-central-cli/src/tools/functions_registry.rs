use burn_central_client::schemas::RegisteredFunction;
pub use inventory;
use quote::ToTokens;

#[derive(Clone, Debug)]
pub struct FunctionMetadata {
    pub mod_path: &'static str,
    pub fn_name: &'static str,
    pub builder_fn_name: &'static str,
    pub proc_type: &'static str,
    pub token_stream: &'static [u8],
}

pub type LazyValue<T> = once_cell::sync::Lazy<T>;
pub struct Plugin<T: 'static>(pub &'static LazyValue<T>);

inventory::collect!(Plugin<FunctionMetadata>);

// macro that generates a flag with a given type and arbitrary parameters and submits it to the inventory
#[macro_export]
macro_rules! register_flag {
    ($type:ty, $init:expr) => {
        const _: () = {
            #[allow(non_upper_case_globals)]
            static FLAG: $crate::registry::LazyValue<$type> =
                $crate::registry::make_static_lazy(|| $init);

            $crate::registry::inventory::submit!($crate::registry::Plugin(&FLAG));
        };
    };
}

pub struct FunctionRegistry {
    flags: LazyValue<Vec<FunctionMetadata>>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self {
            flags: LazyValue::new(|| {
                inventory::iter::<Plugin<FunctionMetadata>>
                    .into_iter()
                    .map(|plugin| (*plugin.0).to_owned())
                    .collect()
            }),
        }
    }

    pub fn get_flags(&self) -> &[FunctionMetadata] {
        &self.flags
    }

    pub fn get_registered_functions(&self) -> Vec<RegisteredFunction> {
        self.flags
            .iter()
            .map(|flag| {
                // function token stream to readable string
                let itemfn = syn_serde::json::from_slice::<syn::ItemFn>(flag.token_stream)
                    .expect("Should be able to parse token stream.");
                let syn_tree: syn::File = syn::parse2(itemfn.into_token_stream())
                    .expect("Should be able to parse token stream.");
                let code_str = prettyplease::unparse(&syn_tree);
                RegisteredFunction {
                    mod_path: flag.mod_path.to_string(),
                    fn_name: flag.fn_name.to_string(),
                    proc_type: flag.proc_type.to_string(),
                    code: code_str,
                }
            })
            .collect()
    }
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
