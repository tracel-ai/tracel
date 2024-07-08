#[derive(Clone, Debug)]
pub struct Flag {
    pub mod_path: &'static str,
    pub name: &'static str,
}

impl Flag {
    pub fn new(mod_path: &'static str, name: &'static str) -> Self {
        Flag { mod_path, name }
    }
}

pub type LazyValue<T> = once_cell::sync::Lazy<T>;
pub struct Plugin<T: 'static>(pub &'static LazyValue<T>);

inventory::collect!(Plugin<Flag>);

pub const fn make_static_lazy<T: 'static>(func: fn() -> T) -> LazyValue<T> {
    LazyValue::<T>::new(func)
}

pub use gensym;
pub use inventory;
pub use paste;

// macro that generates a flag with a given type and arbitrary parameters and submits it to the inventory
#[macro_export]
macro_rules! register_flag {
    ($a:ty, $fn_:expr) => {
        $crate::registry::gensym::gensym! { $crate::register_flag!{ $a, $fn_ } }
    };
    ($gensym:ident, $a:ty, $fn_:expr) => {
        $crate::registry::paste::paste! {
            static [<$gensym _register_flag_>]: $crate::registry::LazyValue<$a> = $crate::registry::make_static_lazy(|| {
                $fn_
            });
            $crate::registry::inventory::submit!($crate::registry::Plugin(&[<$gensym _register_flag_>]));
        }
    };
}
