use crate::{
    data::{MnistBatch, MnistBatcher},
    model::{Model, ModelConfig},
};
use burn::{
    data::dataset::transform::SamplerDataset, record::HalfPrecisionSettings, train::metric::*,
};
use burn::{
    data::{dataloader::DataLoaderBuilder, dataset::vision::MnistDataset},
    nn::loss::CrossEntropyLossConfig,
    optim::AdamConfig,
    prelude::*,
    tensor::backend::AutodiffBackend,
    train::{
        metric::{AccuracyMetric, LossMetric},
        ClassificationOutput, LearnerBuilder, TrainOutput, TrainStep, ValidStep,
    },
};
use tracel::heat::{client::HeatClient, command::MultiDevice, sdk_cli::macros::heat};

impl<B: Backend> Model<B> {
    pub fn forward_classification(
        &self,
        images: Tensor<B, 3>,
        targets: Tensor<B, 1, Int>,
    ) -> ClassificationOutput<B> {
        let output = self.forward(images);
        let loss = CrossEntropyLossConfig::new()
            .init(&output.device())
            .forward(output.clone(), targets.clone());

        ClassificationOutput::new(loss, output, targets)
    }
}

impl<B: AutodiffBackend> TrainStep<MnistBatch<B>, ClassificationOutput<B>> for Model<B> {
    fn step(&self, batch: MnistBatch<B>) -> TrainOutput<ClassificationOutput<B>> {
        let item = self.forward_classification(batch.images, batch.targets);

        TrainOutput::new(self, item.loss.backward(), item)
    }
}

impl<B: Backend> ValidStep<MnistBatch<B>, ClassificationOutput<B>> for Model<B> {
    fn step(&self, batch: MnistBatch<B>) -> ClassificationOutput<B> {
        self.forward_classification(batch.images, batch.targets)
    }
}

#[derive(Config)]
pub struct TrainingConfig {
    pub model: ModelConfig,
    pub optimizer: AdamConfig,
    #[config(default = 10)]
    pub num_epochs: usize,
    #[config(default = 64)]
    pub batch_size: usize,
    #[config(default = 4)]
    pub num_workers: usize,
    #[config(default = 42)]
    pub seed: u64,
    #[config(default = 1.0e-4)]
    pub learning_rate: f64,
}

fn create_artifact_dir(artifact_dir: &str) {
    // Remove existing artifacts before to get an accurate learner summary
    std::fs::remove_dir_all(artifact_dir).ok();
    std::fs::create_dir_all(artifact_dir).ok();
}

pub fn train<B: AutodiffBackend>(
    client: &mut HeatClient,
    artifact_dir: &str,
    config: TrainingConfig,
    device: B::Device,
) -> Result<Model<B>, ()> {
    create_artifact_dir(artifact_dir);
    config
        .save(format!("{artifact_dir}/config.json"))
        .expect("Config should be saved successfully");

    B::seed(config.seed);

    let batcher_train = MnistBatcher::<B>::new(device.clone());
    let batcher_valid = MnistBatcher::<B::InnerBackend>::new(device.clone());

    let dataloader_train = DataLoaderBuilder::new(batcher_train)
        .batch_size(config.batch_size)
        .shuffle(config.seed)
        .num_workers(config.num_workers)
        .build(SamplerDataset::new(MnistDataset::train(), 100));

    let dataloader_test = DataLoaderBuilder::new(batcher_valid)
        .batch_size(config.batch_size)
        .shuffle(config.seed)
        .num_workers(config.num_workers)
        .build(SamplerDataset::new(MnistDataset::test(), 20));

    let recorder =
        tracel::heat::RemoteRecorder::<HalfPrecisionSettings>::checkpoint(client.clone());
    let train_metric_logger = tracel::heat::metrics::RemoteMetricLogger::new_train(client.clone());
    let valid_metric_logger =
        tracel::heat::metrics::RemoteMetricLogger::new_validation(client.clone());

    let learner = LearnerBuilder::new(artifact_dir)
        .metric_train_numeric(AccuracyMetric::new())
        .metric_valid_numeric(AccuracyMetric::new())
        .metric_train_numeric(LossMetric::new())
        .metric_valid_numeric(LossMetric::new())
        .metric_train_numeric(CpuMemory::new())
        .metric_valid_numeric(CpuMemory::new())
        .metric_loggers(train_metric_logger, valid_metric_logger)
        .with_file_checkpointer(recorder)
        .with_application_logger(Some(Box::new(
            tracel::heat::log::RemoteExperimentLoggerInstaller::new(client.clone()),
        )))
        .devices(vec![device.clone()])
        .num_epochs(config.num_epochs)
        .summary()
        .build(
            config.model.init::<B>(&device),
            config.optimizer.init(),
            config.learning_rate,
        );

    let model_trained = learner.fit(dataloader_train, dataloader_test);

    Ok(model_trained)
}

#[heat(training)]
pub fn training<B: AutodiffBackend>(
    mut client: HeatClient,
    config: TrainingConfig,
    MultiDevice(devices): MultiDevice<B>,
) -> Result<Model<B>, ()> {
    train::<B>(&mut client, "/tmp/guide", config, devices[0].clone())
}

#[heat(training)]
pub fn training2<B: AutodiffBackend>(
    config: TrainingConfig,
    MultiDevice(devices): MultiDevice<B>,
    mut client: HeatClient,
) -> Result<Model<B>, ()> {
    train::<B>(&mut client, "/tmp/guide2", config, devices[0].clone())
}

#[heat(training)]
pub fn custom_training<B: AutodiffBackend>(
    MultiDevice(devices): MultiDevice<B>,
) -> Result<Model<B>, ()> {
    println!("Custom training: {:?}", devices);
    Err(())
}

#[heat(training)]
pub fn nothingburger<B: AutodiffBackend>() -> Result<Model<B>, ()> {
    println!("Nothingburger");
    Err(())
}
