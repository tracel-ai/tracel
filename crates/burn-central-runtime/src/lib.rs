use std::fmt;
use std::rc::Rc;

mod type_name;
mod workflow;

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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.text)
    }
}

// This makes Debuggable have most methods of the thing it wraps.
// It also lets you call it when T is a function.
impl<T: ?Sized> ::std::ops::Deref for Debuggable<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
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

fn i_am_a_function(x: &u32) -> u32 {
    *x
}

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

    schedule.add_systems(
        (
            // system_two.pipe(),
            system_three
                .map(|i: i32| {
                    print!("{i}");
                    3
                })
                .map(|i: i32| print!("{i}"))
                .after(system_two)
        ),
    );

    schedule.run(&mut world);
}
