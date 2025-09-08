use crate::inference::ModelAccessor;
use crate::inference::core::InferenceBuilder;
use crate::inference::error::InferenceError;
use crate::inference::init::Init;
use crate::inference::streaming::OutStream;
use crate::{In, MultiDevice, Out, State};
use burn::backend::NdArray;
use burn::config::Config;
use burn::nn::{Linear, LinearConfig};
use burn::prelude::{Backend, Module};
use burn::record::{FullPrecisionSettings, NamedMpkBytesRecorder, Recorder};
use burn::tensor::Tensor;

type TestBackend = NdArray;
type Device = <TestBackend as Backend>::Device;

#[derive(Config, Debug)]
pub struct TestModelConfig {
    input_size: usize,
    output_size: usize,
}

impl TestModelConfig {
    pub fn init<B: Backend>(&self, device: &B::Device) -> TestModel<B> {
        let linear = LinearConfig::new(self.input_size, self.output_size).init(device);
        TestModel { linear }
    }
}

#[derive(Module, Debug)]
pub struct TestModel<B: Backend> {
    linear: Linear<B>,
}

pub struct TestModelArtifacts {
    pub config: TestModelConfig,
    pub weights: Vec<u8>,
}

impl<B: Backend> Init<B, TestModelArtifacts> for TestModel<B> {
    type Error = anyhow::Error;
    fn init(args: &TestModelArtifacts, device: &B::Device) -> Result<Self, Self::Error> {
        let config = &args.config;
        println!("Loading test model with config: {config:?}");
        let model = config.init(device);

        let recorder = NamedMpkBytesRecorder::<FullPrecisionSettings>::default();
        let model_record = recorder.load(args.weights.clone(), device)?;
        let model = model.load_record(model_record);
        Ok(model)
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

fn create_test_model_artifacts() -> TestModelArtifacts {
    let config = TestModelConfig::new(10, 10);
    let model = config.init::<TestBackend>(&Device::default());

    let recorder = NamedMpkBytesRecorder::<FullPrecisionSettings>::default();
    let record = model.into_record();
    let weights = recorder.record(record, ()).unwrap();
    TestModelArtifacts { config, weights }
}

#[test]
fn test_streaming_inference() {
    let artifacts = create_test_model_artifacts();
    let device = Device::default();

    let inference = InferenceBuilder::<TestBackend>::new()
        .init(&artifacts, &device)
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
    let artifacts = create_test_model_artifacts();
    let device = Device::default();

    let inference = InferenceBuilder::<TestBackend>::new()
        .init(&artifacts, &device)
        .unwrap()
        .build(direct_inference_handler);

    let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
    let output = inference.infer(input).with_devices([device]).run().unwrap();

    assert_eq!(output.len(), 1);
    println!("Direct inference completed");
}

#[test]
fn test_stateful_inference() {
    let artifacts = create_test_model_artifacts();
    let device = Device::default();

    let inference = InferenceBuilder::<TestBackend>::new()
        .init(&artifacts, &device)
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
    let artifacts = create_test_model_artifacts();
    let device = Device::default();

    let inference = InferenceBuilder::<TestBackend>::new()
        .init(&artifacts, &device)
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
    assert!(matches!(result, Err(InferenceError::Cancelled)));
    match result {
        Ok(_) => println!("Job completed successfully"),
        Err(InferenceError::Cancelled) => println!("Job was cancelled as expected"),
        Err(e) => panic!("Unexpected error: {}", e),
    }

    assert_eq!(output_count, 2);
}
