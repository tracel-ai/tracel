use std::fmt;
use std::rc::Rc;

mod workflow;
mod type_name;

/// Extends a (possibly unsized) value with a Debug string.
// (This type is unsized when T is unsized)
pub struct Debuggable<T: ?Sized> {
    text: &'static str,
    value: T,
}

/// Produce a Debuggable<T> from an expression for T
macro_rules! dbg {
    ($($body:tt)+) => {
        Debuggable {
            text: stringify!($($body)+),
            value: $($body)+,
        }
    };
}

// Note: this type is unsized
pub type PolFn = Debuggable<dyn Fn(i32) -> i32>;

impl<T: ?Sized> fmt::Debug for Debuggable<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result
    { write!(f, "{}", self.text) }
}

// This makes Debuggable have most methods of the thing it wraps.
// It also lets you call it when T is a function.
impl<T: ?Sized> ::std::ops::Deref for Debuggable<T>
{
    type Target = T;
    fn deref(&self) -> &T { &self.value }
}

fn test_fn(x: i32) -> i32 {
    let _ = "random code so you can see how it's formatted";
    assert_eq!(3 * (1 + 2), 9);
    x
}

#[test]
fn mai2n() {
    let d: &PolFn = &dbg!(test_fn);
    println!("{:?}", d);
}

fn i_am_a_function(x: &u32) -> u32 { *x }

// example of a type storing a debuggable fn

#[test]
fn main() {
    use bevy_ecs::prelude::{ResMut, Resource};
    #[derive(Debug, Resource)]
    struct A {
        func: Debuggable<fn(&u32) -> u32>,
    }




    fn system_two() -> Result<()> {
        let a = A {
            func: dbg!(i_am_a_function),
        };
        println!("System 2 works! {:?}", a.func);
        Ok(())
    }
    fn system_three(res: ResMut<A>) -> i32 {
        println!("System 3 works!");
        3
    }


    use bevy_ecs::prelude::*;
    let mut world = World::new();
    let mut schedule = Schedule::default();

    schedule.add_systems((
        // system_two.pipe(),
        system_three.map(|i: i32| {
            print!("{i}");
            3
        }).map(|i: i32| print!("{i}")).after(system_two)
    ));

    schedule.run(&mut world);
}

mod backend_test {
    use burn::prelude::Backend;
    use burn::tensor::backend::AutodiffBackend;

    pub trait ExperimentHandler<B: Backend> {
        fn run(&self, backend: B::Device);
    }

    pub trait AutodiffExperimentHandler<B: AutodiffBackend> {
        fn run_autodiff(&self, backend: B::Device);
    }


    pub trait HandlerMarker {}
    pub struct Base;
    pub struct RequiresAutodiff;

    impl HandlerMarker for Base {}
    impl HandlerMarker for RequiresAutodiff {}


    pub struct HandlerWrapper<T, M> {
        inner: T,
        _marker: std::marker::PhantomData<M>,
    }

    impl<T, M> HandlerWrapper<T, M> {
        pub fn new(inner: T) -> Self {
            Self { inner, _marker: std::marker::PhantomData }
        }
    }

    pub trait RunHandler<B: Backend> {
        fn run(&self, backend: B::Device);
    }

    impl<T, B> RunHandler<B> for HandlerWrapper<T, Base>
    where
        B: Backend,
        T: ExperimentHandler<B>,
    {
        fn run(&self, backend: B::Device) {
            self.inner.run(backend)
        }
    }

    impl<T, B> RunHandler<B> for HandlerWrapper<T, RequiresAutodiff>
    where
        B: AutodiffBackend,
        T: AutodiffExperimentHandler<B>,
    {
        fn run(&self, backend: B::Device) {
            self.inner.run_autodiff(backend)
        }
    }


    use std::collections::HashMap;

    pub struct HandlerRegistry<B: Backend> {
        handlers: HashMap<String, Box<dyn RunHandler<B>>>,
    }

    impl<B: Backend> HandlerRegistry<B> {
        pub fn new() -> Self {
            Self {
                handlers: HashMap::new(),
            }
        }

        pub fn register<H>(&mut self, name: &str, handler: H)
        where
            H: RunHandler<B> + 'static,
        {
            self.handlers.insert(
                name.to_string(),
                Box::new(handler)
            );
        }

        pub fn run(&self, name: &str, backend: B::Device) {
            if let Some(func) = self.handlers.get(name) {
                func.run(backend);
            } else {
                println!("No handler found for name: {name}");
            }
        }
    }


    mod test_api {
        use burn::backend::{Autodiff, NdArray};
        use burn::prelude::Backend;
        use crate::backend_test::{AutodiffExperimentHandler, ExperimentHandler};


    }

}