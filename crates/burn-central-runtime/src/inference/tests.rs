#[cfg(test)]
mod tests {
    use crate::inference::core::InferenceBuilder;
    use crate::inference::emitter::OutStream;
    use crate::inference::errors::{InferenceError, ModelProviderResult};
    use crate::inference::provider::ModelProvider;
    use crate::model::ModelAccessor;
    use crate::{In, MultiDevice, Out, State};
    use burn::backend::NdArray;
    use burn::nn::{Linear, LinearConfig};
    use burn::prelude::{Backend, Module};
    use burn::tensor::Tensor;
    use burn_central_client::credentials::BurnCentralCredentials;
    use burn_central_client::model::{ModelRegistry, ModelSpec};
    use std::str::FromStr;

    type TestBackend = NdArray;
    type Device = <TestBackend as Backend>::Device;

    #[derive(Module, Debug)]
    pub struct TestModel<B: Backend> {
        linear: Linear<B>,
    }

    impl<B: Backend> TestModel<B> {
        fn new(device: &B::Device) -> Self {
            let linear = LinearConfig::new(10, 5).init(device);
            TestModel { linear }
        }
    }

    impl<B: Backend> ModelProvider<B> for TestModel<B> {
        fn get_model(
            _registry: &ModelRegistry,
            model_spec: ModelSpec,
            device: &B::Device,
        ) -> ModelProviderResult<Self> {
            println!("Loading test model with spec: {model_spec}");
            Ok(TestModel::new(device))
        }
    }

    fn streaming_inference_handler<B: Backend>(
        In(input): In<Tensor<B, 2>>,
        model: ModelAccessor<TestModel<B>>,
        devices: MultiDevice<B>,
        output: OutStream<Tensor<B, 2>>,
    ) -> Result<(), InferenceError> {
        println!("Using device: {:?}", devices[0]);
        let mut result = input;
        for i in 0..3 {
            result = model.with(move |m| m.linear.forward(result.clone() + i));
            if output.emit(result.clone()).is_err() {
                return Err(InferenceError::Cancelled);
            }
        }
        Ok(())
    }

    fn direct_inference_handler<B: Backend>(
        In(input): In<Tensor<B, 2>>,
        model: ModelAccessor<TestModel<B>>,
        _devices: MultiDevice<B>,
    ) -> Out<Tensor<B, 2>> {
        let result = model.with(move |m| m.linear.forward(input));
        result.into()
    }

    fn stateful_inference_handler<B: Backend>(
        In(input): In<Tensor<B, 2>>,
        model: ModelAccessor<TestModel<B>>,
        _devices: MultiDevice<B>,
        output: OutStream<String>,
        State(counter): State<i32>,
    ) -> Result<(), InferenceError> {
        let result = model.with(move |m| m.linear.forward(input));
        for i in 0..counter {
            if output
                .emit(format!("Step {}: {:?}", i, result.to_data()))
                .is_err()
            {
                return Err(InferenceError::Cancelled);
            }
        }
        Ok(())
    }

    fn create_test_client() -> burn_central_client::BurnCentral {
        let creds = BurnCentralCredentials::from_str("test-credentials")
            .expect("Should create test credentials");

        burn_central_client::BurnCentral::builder(creds)
            .with_endpoint("http://localhost:9001")
            .build()
            .expect("Should create test client")
    }

    #[test]
    fn test_streaming_inference() {
        let client = create_test_client();
        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client)
            .load::<TestModel<TestBackend>>("test/model:1".parse().unwrap(), &device)
            .unwrap()
            .build(streaming_inference_handler);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        let output = inference.infer(input).with_devices([device]).run().unwrap();

        assert_eq!(output.len(), 3);
        println!(
            "Streaming inference completed with {} outputs",
            output.len()
        );
    }

    #[test]
    fn test_direct_inference() {
        let client = create_test_client();
        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client)
            .load::<TestModel<TestBackend>>("test/model:1".parse().unwrap(), &device)
            .unwrap()
            .build(direct_inference_handler);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        let output = inference.infer(input).with_devices([device]).run().unwrap();

        assert_eq!(output.len(), 1);
        println!("Direct inference completed");
    }

    #[test]
    fn test_stateful_inference() {
        let client = create_test_client();
        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client)
            .load::<TestModel<TestBackend>>("test/model:1".parse().unwrap(), &device)
            .unwrap()
            .build(stateful_inference_handler);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        let output = inference
            .infer(input)
            .with_devices([device])
            .with_state(2)
            .run()
            .unwrap();

        assert_eq!(output.len(), 2);
        println!("Stateful inference completed with state");
    }

    #[test]
    fn test_async_inference_job() {
        let client = create_test_client();
        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client)
            .load::<TestModel<TestBackend>>("test/model:1".parse().unwrap(), &device)
            .unwrap()
            .build(streaming_inference_handler);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        let job = inference.infer(input).with_devices([device]).spawn();

        let mut output_count = 0;
        for output in job.stream.iter() {
            println!("Received output: {:?}", output.to_data());
            output_count += 1;
            if output_count >= 2 {
                job.cancel();
                break;
            }
        }

        let result = job.join();
        match result {
            Ok(_) => println!("Job completed successfully"),
            Err(InferenceError::Cancelled) => println!("Job was cancelled as expected"),
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }
}
