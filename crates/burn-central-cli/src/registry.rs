#[derive(Clone, Debug)]
pub struct Flag {
    pub mod_path: &'static str,
    pub fn_name: &'static str,
    pub proc_type: &'static str,
    pub token_stream: &'static [u8],
}

impl Flag {
    pub fn new(
        mod_path: &'static str,
        fn_name: &'static str,
        proc_type: &'static str,
        token_stream: &'static [u8],
    ) -> Self {
        Flag {
            mod_path,
            fn_name,
            proc_type,
            token_stream,
        }
    }
}

pub type LazyValue<T> = once_cell::sync::Lazy<T>;
pub struct Plugin<T: 'static>(pub &'static LazyValue<T>);

inventory::collect!(Plugin<Flag>);

pub const fn make_static_lazy<T: 'static>(func: fn() -> T) -> LazyValue<T> {
    LazyValue::<T>::new(func)
}

pub use inventory;
pub use paste;

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

pub(crate) fn get_flags() -> Vec<Flag> {
    inventory::iter::<Plugin<Flag>>
        .into_iter()
        .map(|plugin| (*plugin.0).to_owned())
        .collect()
}
