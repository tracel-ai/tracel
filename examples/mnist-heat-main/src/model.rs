//! This module explores a new way of defining trainable modules using a macro that generates the necessary boilerplate code for training functions.
//!
//! This approach centers around the `Module<B>` trait, which is implemented for a custom model struct.
//! The `TrainableModule<B>` trait is defined to allow the module to be trainable with CLI commands.
//! It can be derived using the `#[derive(TrainableModule)]` macro, which generates the necessary code to register training functions.
//!
//! The `TrainGroup<B>` trait is also defined to group training functions and provide a way to call them based on their names.
//! This trait would be most likely hidden from the user, as it is used internally by the macro to generate the train functions.
//!
//! To register training functions, the user can use the `#[train_impl]` macro on their model struct's impl block, which will generate the necessary code to call the training functions based on their names.
//! Additionally, if the user wants to define multiple impl blocks for different training functions, they can tag them with `#[train_impl(ModelTrainGroup)]` to indicate that they belong to the same group of training functions.
//! Then the model struct also needs to be tagged with the `#[train_impl(ModelTrainGroup)]` macro so that the macro can generate the necessary code to define the train groups.
//!
//! Demo:
//! ```rust,ignore
//!
//!  #[derive(Module, TrainableModule)]
//!  #[train_impl(Group1, Group2)]
//!  pub struct Model<B: Backend> {
//!     conv1: Conv2d<B>,
//!     // ...
//!  }
//!
//!  #[train_impl(Group1)]
//!  impl<B: AutodiffBackend> Model<B> {
//!     pub fn train1() -> Result<Self, TrainingError> {
//!      // Custom training logic for train1
//!     }
//!  }
//!
//!  #[train_impl(Group2)]
//!  impl<B: AutodiffBackend> Model<B> {
//!     pub fn train1() -> Result<Self, TrainingError> {
//!      // Custom training logic for train1
//!     }
//!  }
//!

mod api {
    use burn::module::Module;
    use burn::prelude::Backend;
    use burn::tensor::backend::AutodiffBackend;
    use tracel::heat::command::TrainCommandContext;

    /// Trait for trainable modules, used by the derive macro to implement the train functions.
    pub trait TrainableModule<B: AutodiffBackend>: Module<B> {
        fn all_train_fn_names() -> Vec<String>;

        fn call_train_fn(
            _name: &str,
            _ctx: TrainCommandContext<B>,
        ) -> Result<Self, Box<dyn std::error::Error>>
        where
            Self: Sized;
    }

    pub trait HashedAssociatedType<const H: u64> {
        type Inner;

        fn get() -> Self::Inner;
    }

    /// A macro that simplifies the hashing of train group names.
    macro_rules! train_impl {
        ($($group:ident),*) => {
            $(
                impl<B: AutodiffBackend> HashedAssociatedType<{ train_group_name_hash(stringify!($group)) }> for $group<B> {
                    type AssociatedType = $group<B>;
                }
            )*
        };
    }

    /// A macro that simplifies getting the associated type of a train group by its name.
    pub fn get_train_group_by_name<const H: u64, T: HashedAssociatedType<H>>(
        _name: &str,
    ) -> T::Inner {
        <T as HashedAssociatedType<H>>::get()
    }


    pub const fn train_group_name_hash(name: &str) -> u64 {
        let bytes = name.as_bytes();
        let mut hash = 0xcbf29ce484222325u64;
        let mut i = 0;
        while i < bytes.len() {
            hash ^= bytes[i] as u64;
            hash = hash.wrapping_mul(0x100000001b3);
            i += 1;
        }
        hash
    }

    /// Internal trait for trainable modules, used by the derive macro to implement the train functions.
    #[diagnostic::on_unimplemented(
        message = "Did you forget to use the **`#[train_impl]`** macro?",
        label = "Here, the `TrainGroup` trait is expected to be implemented.",
        note = "The `TrainGroup` trait is automatically implemented by the `#[train_impl]` macro."
    )]
    pub trait TrainGroup<B: AutodiffBackend> {
        type InternalModule: Module<B>;

        const HACK: ();

        /// Returns the name of the train function.
        fn all_train_fn_names(&self) -> Vec<String> {
            vec![]
        }

        fn call_train_fn(
            &self,
            _name: &str,
            _ctx: TrainCommandContext<B>,
        ) -> Result<Self::InternalModule, Box<dyn std::error::Error>>
        where
            Self: Sized,
        {
            Err("`#[derive(Trainable)]` used, but no #[train_impl] block found.".into())
        }
    }
}

use crate::model::api::{train_group_name_hash, HashedAssociatedType, TrainGroup, TrainableModule};
use burn::module::AutodiffModule;
use burn::tensor::backend::AutodiffBackend;
use burn::{
    nn::{
        conv::{Conv2d, Conv2dConfig},
        pool::{AdaptiveAvgPool2d, AdaptiveAvgPool2dConfig},
        Dropout, DropoutConfig, Linear, LinearConfig, Relu,
    },
    prelude::*,
};
use std::error::Error;
use std::marker::PhantomData;
use tracel::heat::client::{HeatClient, HeatClientConfig, HeatCredentials};
use tracel::heat::command::{TrainCommandContext, TrainCommandHandler};
use tracel::heat::errors::training::TrainingError;
use tracel::heat::schemas::ProjectPath;

/// Deriving the TrainableModule trait allows the module to be trainable with CLI commands.
#[derive(Module, Debug /*TrainableModule*/)]
/// The impl block needs to be associated with the module to allow the macro to generate the train functions.
/// If none is provided, it will use the default `ModelTrainGroup` generated by the macro.
/// #[impl(ModelTrainGroup, ModelTrainGroup2)]
pub struct Model<B: Backend> {
    conv1: Conv2d<B>,
    conv2: Conv2d<B>,
    pool: AdaptiveAvgPool2d,
    dropout: Dropout,
    linear1: Linear<B>,
    linear2: Linear<B>,
    activation: Relu,
}

/// Generated by derive macro
/// This struct serves as an intermediate type to allow the `#[train_impl]` macro to generate the train functions.
/// #[impl(ModelTrainGroup)]
pub struct ModelTrainGroup<B: AutodiffBackend> {
    _phantom: PhantomData<B>,
}
pub struct OtherModelTrainGroup2;

impl<B: AutodiffBackend> ModelTrainGroup<B>
{
    pub fn new() -> Self {
        ModelTrainGroup {
            _phantom: PhantomData,
        }
    }

    // const HACK: () = {
    //     panic!("This is a compile-time panic to ensure the macro is used correctly.");
    // };

    /// Returns the names of all train functions.
    pub fn all_train_fn_names(&self) -> Vec<String> {
        vec![]
    }

    fn call_train_fn(
        &self,
        _: &str,
        _: TrainCommandContext<B>,
    ) -> Result<Model<B>, Box<dyn Error>> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use burn::backend::{Autodiff, Wgpu};
    use crate::model::api::{train_group_name_hash, HashedAssociatedType, TrainGroup};
    use crate::model::{Model};

    type TG = <Model<Autodiff<Wgpu>> as HashedAssociatedType<{
        train_group_name_hash(stringify!(ModelTrainGroup))
    }>>::Inner;
    #[test]
    fn test_model_train_group() {
        let group = TG::new();
        let a = <TG as TrainGroup<Autodiff<Wgpu>>>::HACK;
        assert!(group.all_train_fn_names().is_empty());
    }

    #[test]
    fn test() {
        _ = TG::HACK;
        TG::new()
            .all_train_fn_names()
            .iter()
            .for_each(|s| println!("{}", s));
        if TG::new().all_train_fn_names().is_empty() {
            panic!("No train functions registered. Did you forget to use the #[train_impl] macro?");
        }
    }
}

/// Generated by derive macro
impl<B: AutodiffBackend> TrainableModule<B> for Model<B> {
    fn all_train_fn_names() -> Vec<String> {
        // ModelTrainGroup.all_train_fn_names()
        unimplemented!()
    }

    fn call_train_fn(
        name: &str,
        ctx: TrainCommandContext<B>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        ModelTrainGroup::<B>::new().call_train_fn(name, ctx)
    }
}

impl<B: AutodiffBackend> Model<B>
where
    Self: TrainableModule<B>,
{
    /// This function is used to select the train function based on the name.
    // const MODULE_TRAIN_GROUP: ModelTrainGroup = ModelTrainGroup;
    const PANIC: () = panic!("compile-time panic");
}

impl<B: AutodiffBackend> HashedAssociatedType<{ train_group_name_hash(stringify!(ModelTrainGroup)) }> for Model<B> {
    type Inner = ModelTrainGroup<B>;

    fn get() -> Self::Inner {
        ModelTrainGroup::new()
    }
}

// impl<B: AutodiffBackend> HashedAssociatedType<1> for Model<B> {
//     type AssociatedType<T: AutodiffBackend> = OtherModelTrainGroup2;
// }

mod test {
    use crate::model::api::{train_group_name_hash, HashedAssociatedType, TrainableModule};
    use crate::model::{Model, TrainGroup};
    use crate::training::{train, TrainingConfig};
    use burn::module::Module;
    use burn::prelude::Backend;
    use burn::tensor::backend::AutodiffBackend;
    use tracel::heat::client::HeatClient;
    use tracel::heat::command::{MultiDevice, TrainCommandContext, TrainCommandHandler};
    use tracel::heat::errors::training::TrainingError;

    pub(crate) fn trigger<
        B: Backend,
        T,
        M: Module<B>,
        E: Into<Box<dyn std::error::Error>>,
        H: TrainCommandHandler<B, T, M, E>,
    >(
        handler: H,
        context: TrainCommandContext<B>,
    ) -> Result<M, Box<dyn std::error::Error>> {
        match handler.call(context) {
            Ok(model) => Ok(model),
            Err(e) => Err(e.into()),
        }
    }

    mod train_group_impl {
        use burn::tensor::backend::AutodiffBackend;
        use tracel::heat::command::TrainCommandContext;
        use tracel::heat::errors::training::TrainingError;
        use crate::model::api::{train_group_name_hash, HashedAssociatedType, TrainGroup};
        use crate::model::Model;

        /// Generated by train_impl macro
        #[diagnostic::do_not_recommend]
        impl<B: AutodiffBackend> TrainGroup<B> for <Model<B> as HashedAssociatedType<{ train_group_name_hash(stringify!(ModelTrainGroup)) }>>::Inner {
            type InternalModule = Model<B>;

            const HACK: () = {
                // panic!("Th")
            };

            fn all_train_fn_names(&self) -> Vec<String> {
                vec!["train1".to_string(), "train2".to_string()]
            }

            fn call_train_fn(
                &self,
                name: &str,
                context: TrainCommandContext<B>,
            ) -> Result<Self::InternalModule, Box<dyn std::error::Error>> {
                match name {
                    "train1" => crate::model::test::trigger(Model::<B>::train1, context),
                    "train2" => crate::model::test::trigger(Model::<B>::train2, context),
                    _ => Err(Box::new(TrainingError::UnknownError(format!(
                        "Unknown train function: {}",
                        name
                    )))),
                }
            }
        }
    }


    /// registered functions impl block
    /// this macro will register the functions in registries
    /// fucntions are grouped by their backend trait bounds
    /// #[train_impl(ModelTrainGroup)]
    /// Where block generated by the `#[train_impl]` macro.
    impl<B: AutodiffBackend> Model<B>
    where
        Self: TrainableModule<B>,
    {
        /// Function that doesn't match, it will not be registered.
        pub fn hello_world() -> String {
            // <ModelTrainGroup as TrainGroup>::all_train_fn_names();
            "Hello, world!".to_string()
        }

        pub fn train1(
            mut client: HeatClient,
            config: TrainingConfig,
            MultiDevice(devices): MultiDevice<B>,
        ) -> Result<Self, TrainingError> {
            _ = <<Model<B> as HashedAssociatedType<{ train_group_name_hash(stringify!(ModelTrainGroup)) }>>::Inner>::HACK;

            println!("custom model train function called");
            let model = train(&mut client, "hi", config, devices[0].clone());
            model
        }

        pub fn train2() -> Result<Self, TrainingError> {
            Err(TrainingError::UnknownError("hi".to_string()))
        }

        // This function is generated by the macro and will be used to select the train function based on the name.
    }
}

#[test]
fn test_model_forward() {
    use burn::backend::wgpu::WgpuDevice;
    let device = WgpuDevice::default();

    let model_config = ModelConfig {
        num_classes: 10,
        hidden_size: 128,
        dropout: 0.5,
    };
    // let model_: Model<Autodiff<Wgpu>> = model_config.init(&device);
    // <Model<Autodiff<Wgpu>> as AutodiffModule<Autodiff<Wgpu>>>::valid(&model_);

    let client = HeatClientConfig::builder(
        HeatCredentials::new("da".parse().unwrap()),
        ProjectPath::try_from("ad".to_string()).unwrap(),
    )
    .build();
    let client = HeatClient::create(client).unwrap();
    // let model = Model::<Autodiff<Wgpu>>::__select_train_function(
    //     "train1",
    //     TrainCommandContext::new(client, vec![device.clone()], model_config.to_string()),
    // );
    // assert!(model.is_ok());
}

#[derive(Config, Debug)]
pub struct ModelConfig {
    num_classes: usize,
    hidden_size: usize,
    #[config(default = "0.5")]
    dropout: f64,
}

impl ModelConfig {
    /// Returns the initialized model.
    pub fn init<B: Backend>(&self, device: &B::Device) -> Model<B> {
        Model {
            conv1: Conv2dConfig::new([1, 8], [3, 3]).init(device),
            conv2: Conv2dConfig::new([8, 16], [3, 3]).init(device),
            pool: AdaptiveAvgPool2dConfig::new([8, 8]).init(),
            activation: Relu::new(),
            linear1: LinearConfig::new(16 * 8 * 8, self.hidden_size).init(device),
            linear2: LinearConfig::new(self.hidden_size, self.num_classes).init(device),
            dropout: DropoutConfig::new(self.dropout).init(),
        }
    }
}

impl<B: Backend> Model<B> {
    /// # Shapes
    ///   - Images [batch_size, height, width]
    ///   - Output [batch_size, class_prob]
    pub fn forward(&self, images: Tensor<B, 3>) -> Tensor<B, 2> {
        let [batch_size, height, width] = images.dims();

        // Create a channel.
        let x = images.reshape([batch_size, 1, height, width]);

        let x = self.conv1.forward(x); // [batch_size, 8, _, _]
        let x = self.dropout.forward(x);
        let x = self.conv2.forward(x); // [batch_size, 16, _, _]
        let x = self.dropout.forward(x);
        let x = self.activation.forward(x);

        let x = self.pool.forward(x); // [batch_size, 16, 8, 8]
        let x = x.reshape([batch_size, 16 * 8 * 8]);
        let x = self.linear1.forward(x);
        let x = self.dropout.forward(x);
        let x = self.activation.forward(x);

        self.linear2.forward(x) // [batch_size, num_classes]
    }
}
