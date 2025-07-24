mod executor {
    use std::marker::PhantomData;
    use std::ops::Deref;
    use anyhow::Result;
    use burn::prelude::{Backend, Module};
    use burn_central_client::command::MultiDevice;
    // Using anyhow for simpler error handling

    // 1. Define your initial "Global" Context (if needed, can be captured by systems)
    // For now, let's assume some global configuration is available implicitly
    // or can be passed to the scheduler/executor.

    // 2. Define your Mutable "Local" Execution Context
    // This struct holds the data that handlers will read from and write to.
    #[derive(Debug, Default)]
    pub struct ExecutionContext<B: Backend> {
        pub devices: Vec<B::Device>,
        pub experiment_data: String,
        pub step_results: Vec<f32>,
        // Add any other data that needs to be modified or passed along the chain
        // For example, paths to intermediate files, model parameters being optimized, etc.
        pub current_model_path: Option<String>,
        pub _backend: PhantomData<B>, // Placeholder for the backend type
    }

    // 3. SystemParam Trait (adapted)
    // How handlers get their arguments from the mutable ExecutionContext
    pub trait SystemParam<B: Backend> {
        type Item<'new>;

        // This method will now take a mutable reference to the ExecutionContext
        fn retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Self::Item<'r>;
    }

    pub trait IntoSystem<Input, Output, B: Backend> {
        type System: System<B>;

        // This method converts a function or closure into a System
        fn into_system(self) -> Self::System;
    }

    // Implement SystemParam for direct access to mutable fields in ExecutionContext
    // Example: Allowing handlers to get a mutable reference to experiment_data
    impl<'ctx, B: Backend> SystemParam<B> for &'ctx String {
        type Item<'new> = &'new String;

        fn retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Self::Item<'r> {
            &ctx.experiment_data
        }
    }

    // Example: Allowing handlers to get a mutable reference to step_results
    impl<'ctx, B: Backend> SystemParam<B> for &'ctx Vec<f32> {
        type Item<'new> = &'new Vec<f32>;

        fn retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Self::Item<'r> {
            &ctx.step_results
        }
    }

    // Example: Allowing handlers to get a mutable reference to current_model_path
    impl<'ctx, B: Backend> SystemParam<B> for &'ctx Option<String> {
        type Item<'new> = &'new Option<String>;

        fn retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Self::Item<'r> {
            &ctx.current_model_path
        }
    }

    impl<'ctx, B: Backend> SystemParam<B> for &'ctx ExecutionContext<B> {
        type Item<'new> = &'new ExecutionContext<B>;

        fn retrieve(ctx: &ExecutionContext<B>) -> Self::Item<'_> {
            ctx
        }
    }


    impl<B: Backend> SystemParam<B> for MultiDevice<B> {
        type Item<'new> = MultiDevice<B>;
        fn retrieve(ctx: &ExecutionContext<B>) -> Self::Item<'_> {
            MultiDevice(ctx.devices.clone())
        }
    }

    pub trait System<B: Backend>: Send + Sync { // Added Send + Sync for potential threading later
        fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<()>;
    }

    /// This trait defines how a specific return type (Output) from a handler
    /// is processed and potentially stored back into the ExecutionContext.
    pub trait IntoSystemOutput<B: Backend>: Send + Sync + 'static {
        /// This method takes the owned output and the mutable ExecutionContext,
        /// allowing the output to modify the context.
        fn apply_output(self: Box<Self>, ctx: &mut ExecutionContext<B>) -> Result<()>;
    }

    impl<B: Backend> IntoSystemOutput<B> for () {
        fn apply_output(self: Box<Self>, _ctx: &mut ExecutionContext<B>) -> Result<()> {
            Ok(()) // Do nothing, successful operation.
        }
    }

    impl<T, E, B: Backend> IntoSystemOutput<B> for core::result::Result<T, E>
    where
        T: IntoSystemOutput<B>,
        E: std::fmt::Display + Send + Sync + 'static
    {
        fn apply_output(self: Box<Self>, ctx: &mut ExecutionContext<B>) -> Result<()> {
            match *self {
                Ok(output) => Box::new(output).apply_output(ctx),
                Err(e) => Err(anyhow::anyhow!("Error applying output: {}", e)),
            }
        }
    }

    pub struct Model<B: Backend, M: Module<B>> {
        pub model: M,
        pub _backend: PhantomData<B>,
    }

    impl<B: Backend, M: Module<B>> Deref for Model<B, M> {
        type Target = M;

        fn deref(&self) -> &Self::Target {
            &self.model
        }
    }

    impl<B: Backend, M: Module<B>> From<M> for Model<B, M> {
        fn from(model: M) -> Self {
            Model {
                model,
                _backend: PhantomData,
            }
        }
    }

    impl<B: Backend, M: Module<B> + Sync + 'static> IntoSystemOutput<B> for Model<B, M> {
        fn apply_output(self: Box<Self>, ctx: &mut ExecutionContext<B>) -> Result<()> {
            // Here we could save the model to a file or update the context
            // For simplicity, let's just print a message
            println!("Model applied to context: {:?}", self.model);
            Ok(())
        }
    }

    pub trait FunctionSystemRun<Input, R, F> {
        fn call_handler<B: Backend>(&self, ctx: &mut ExecutionContext<B>) -> R;
    }

    pub struct TrainingStep<F>(pub F);

    pub struct WrappedTrainingSystem<I, F, M> {
        inner: FunctionSystem<I, Result<M>, F>,
    }

// Macro to implement TrainingStep for different parameter counts
    macro_rules! impl_training_step {
        ($($T:ident),*) => {
            impl<F, B: Backend, M: Module<B> + Sync + 'static, $($T: SystemParam<B>),*> IntoSystem<($($T,)*), Result<M>, B> for TrainingStep<F>
            where
                for<'a, 'b> &'a F:
                    Fn($($T),*) -> Result<M> +
                    Fn($(<$T as SystemParam<B>>::Item<'b>),*) -> Result<M>,
                F: Fn($($T),*) -> Result<M> + Send + Sync + 'static,
            {
                type System = WrappedTrainingSystem<($($T,)*), F, M>;

                fn into_system(self) -> Self::System {
                    WrappedTrainingSystem {
                        inner: FunctionSystem {
                            f: self.0,
                            marker: PhantomData,
                        },
                    }
                }
            }

            #[allow(unused_variables)]
            #[allow(non_snake_case)]
            impl<F, B: Backend, M: Module<B> + Sync + 'static, $($T: SystemParam<B>),*> System<B> for WrappedTrainingSystem<($($T,)*), F, M>
            where
                for<'a, 'b> &'a F:
                    Fn($($T),*) -> Result<M> +
                    Fn($(<$T as SystemParam<B>>::Item<'b>),*) -> Result<M>,
                F: Fn($($T),*) -> Result<M> + Send + Sync + 'static,
            {
                fn run(&self, _ctx: &mut ExecutionContext<B>) -> Result<()> {
                    fn call_inner<M, $($T),*>(
                        f: impl Fn($($T),*) -> Result<M>,
                        $($T: $T),*
                    ) -> Result<M>
                    {
                        f($($T),*)
                    }

                    $(
                        let $T = <$T as SystemParam<B>>::retrieve(_ctx);
                    )*
                    let output = call_inner(&self.inner.f, $($T),*)?;
                    Box::new(Model::<B, M>::from(output)).apply_output(_ctx)
                }
            }
        };
    }

    // Implement for different numbers of arguments
    impl_training_step!();
    impl_training_step!(P1);
    impl_training_step!(P1, P2);
    impl_training_step!(P1, P2, P3);
    impl_training_step!(P1, P2, P3, P4);

    // To support functions/closures as systems
    pub struct FunctionSystem<Input, Output, F> {
        f: F,
        marker: PhantomData<fn() -> (Input, Output)>,
    }

    // This macro helps implement IntoSystem and System for various function arities
    macro_rules! impl_system_for_fn {
        ($($T:ident),*) => {

            #[allow(unused_variables)]
            #[allow(non_snake_case)]
            impl<F, R, B: Backend, $($T: SystemParam<B>),*> IntoSystem<($($T,)*), R, B> for F
            where
                for<'a, 'b> &'a F:
                    Fn($($T),*) -> R +
                    Fn($(<$T as SystemParam<B>>::Item<'b>),*) -> R,
                F: Fn($($T),*) -> R + Send + Sync + 'static,
                R: IntoSystemOutput<B>,
            {
                type System = FunctionSystem<($($T,)*), R, Self>;

                fn into_system(self) -> Self::System {
                    FunctionSystem {
                        f: self,
                        marker: PhantomData,
                    }
                }
            }

            #[allow(unused_variables)]
            #[allow(non_snake_case)]
            impl<F, R, B: Backend, $($T: SystemParam<B>),*> System<B> for FunctionSystem<($($T,)*), R, F>
            where
                for<'a, 'b> &'a F:
                    Fn($($T),*) -> R +
                    Fn($(<$T as SystemParam<B>>::Item<'b>),*) -> R,
                F: Fn($($T),*) -> R + Send + Sync + 'static,
                R: IntoSystemOutput<B>,
            {
                fn run(&self, _ctx: &mut ExecutionContext<B>) -> Result<()> {
                    fn call_inner<R, $($T),*>(
                        f: impl Fn($($T),*) -> R,
                        $($T: $T),*
                    ) -> R
                    {
                        f($($T),*)
                    }

                    $(
                        let $T = <$T as SystemParam<B>>::retrieve(_ctx);
                    )*
                    let output = call_inner(&self.f, $($T),*);
                    Box::new(output).apply_output(_ctx)
                }
            }
        };
    }

    // Implement for different numbers of arguments
    impl_system_for_fn!();
    impl_system_for_fn!(P1);
    impl_system_for_fn!(P1, P2);
    impl_system_for_fn!(P1, P2, P3);
    impl_system_for_fn!(P1, P2, P3, P4);


    // 6. Executor (instead of Scheduler)
    // Manages the chain of handlers for a single run
    pub struct Executor<B: Backend> {
        handlers: Vec<Box<dyn System<B>>>,
    }

    impl<B: Backend> Executor<B> {
        pub fn new() -> Self {
            Executor { handlers: Vec::new() }
        }

        pub fn add_handler<I, O, S: System<B> + 'static>(&mut self, handler: impl IntoSystem<I, O, B, System = S>) {
            self.handlers.push(Box::new(handler.into_system()));
        }

        // This runs a single chain of handlers with an initial context
        pub fn execute(&self, mut initial_context: ExecutionContext<B>) -> Result<ExecutionContext<B>> {
            println!("--- Starting Execution ---");
            println!("Initial Context: {:?}", initial_context);

            for (i, handler) in self.handlers.iter().enumerate() {
                println!("\n--- Running Handler {} ---", i);
                handler.run(&mut initial_context)?;
                println!("Context after Handler {}: {:?}", i, initial_context);
            }

            println!("\n--- Execution Finished ---");
            Ok(initial_context)
        }
    }

    // --- Example Handlers ---

    // Handler that modifies experiment_data
    fn preprocess_data<B: Backend>(data: &String, results: &Vec<f32>) -> Result<()> {
        println!("  Preprocessed data. New data: {}", data);
        Ok(())
    }

    // Handler that reads data and writes a model path
    fn train_model(data: &String, model_path: &Option<String>) -> core::result::Result<(), String> {
        println!("  Training model with data: {}", data);
        // Simulate some training logic
        // *model_path = Some(format!("/models/{}-v1.pkl", data));
        println!("  Model trained. Path: {:?}", model_path);
        Err("hello".into())
    }

    // Handler that uses the model path to evaluate
    fn evaluate_model(model_path: &Option<String>, results: &Vec<f32>) -> Result<()> {
        if let Some(path) = model_path {
            println!("  Evaluating model from path: {}", path);
            // results.push(0.95); // Add a metric
        } else {
            println!("  No model path to evaluate.");
        }
        println!("  Results after evaluation: {:?}", results);
        Ok(())
    }

    // Handler that takes no arguments
    fn log_completion() -> Result<()> {
        println!("  Experiment run completed!");
        Ok(())
    }


    #[test]
    fn test_chained_handlers() {


        #[derive(Module, Debug)]
        pub struct TestModel<B: Backend> {
            // Define your model parameters here
            _backend: PhantomData<B>,
        }

        impl<B: Backend> Default for TestModel<B> {
            fn default() -> Self {
                TestModel {
                    _backend: PhantomData,
                }
            }
        }

        fn log_model<B: Backend>(MultiDevice(devices): MultiDevice<B>) -> Result<TestModel<B>> {
            println!("  Logging model...");
            // Here you would typically save the model to a file or database
            Ok(TestModel::default())
        }

        type Back = burn::backend::NdArray;
        type Device = <Back as Backend>::Device;


        let mut executor = Executor::<Back>::new();

        // Add handlers to the executor
        executor.add_handler(preprocess_data::<Back>);
        executor.add_handler(train_model);
        executor.add_handler(evaluate_model);
        executor.add_handler(TrainingStep(log_model::<Back>));

        // Create an initial context for a specific experiment run
        let initial_ctx = ExecutionContext {
            devices: vec![Device::default()],
            experiment_data: "raw_experiment_A".to_string(),
            step_results: vec![],
            current_model_path: None,
            _backend: Default::default(),
        };

        // Execute the chain
        let final_ctx = executor.execute(initial_ctx).expect("Execution failed");

        println!("\nFinal Context after all handlers: {:?}", final_ctx);
        assert_eq!(final_ctx.experiment_data, "raw_experiment_A-processed");
        assert_eq!(final_ctx.step_results, vec![10.0, 0.95]);
        assert_eq!(final_ctx.current_model_path, Some("/models/raw_experiment_A-processed-v1.pkl".to_string()));
    }
}
