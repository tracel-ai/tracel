use std::sync::Arc;

use crate::{
    data::{MnistBatcher, MnistItemPrepared, MnistMapper, Transform},
    model::{MnistModel, MnistModelArtifact},
};

use burn::optim::AdamWConfig;
use burn::train::Learner;
use burn::{
    data::{
        dataloader::DataLoaderBuilder,
        dataset::{
            Dataset,
            transform::{ComposedDataset, MapperDataset, PartialDataset, SamplerDataset},
            vision::{MnistDataset, MnistItem},
        },
    },
    lr_scheduler::{
        composed::ComposedLrSchedulerConfig, cosine::CosineAnnealingLrSchedulerConfig,
        linear::LinearLrSchedulerConfig,
    },
    prelude::*,
    train::{
        EvaluatorBuilder, LearningResult, MetricEarlyStoppingStrategy, StoppingCondition,
        SupervisedTraining,
        metric::{
            AccuracyMetric, LearningRateMetric, LossMetric,
            store::{Aggregate, Direction, Split},
        },
        renderer::MetricsRenderer,
    },
};
use tracel::experiment::{
    ArtifactKind, ExperimentId, ExperimentRun, integration::training::ExperimentTrainingExt,
};
static ARTIFACT_DIR: &str = "/tmp/burn-example-mnist";

#[derive(Config, Debug)]
pub struct ResumeConfig {
    pub experiment: u32,
    pub epoch: usize,
}

#[derive(Config, Debug)]
pub struct MnistTrainingConfig {
    #[config(default = 20)]
    pub num_epochs: usize,

    #[config(default = 256)]
    pub batch_size: usize,

    #[config(default = 8)]
    pub num_workers: usize,

    #[config(default = 42)]
    pub seed: u64,

    pub optimizer: AdamWConfig,

    /// When set, resume training from a previous experiment's checkpoints
    pub resume_from: Option<ResumeConfig>,
}

/// Implement default training configuration. The tracel-cli will be able to override those
/// value and those not specified will use their default value.
impl Default for MnistTrainingConfig {
    fn default() -> Self {
        Self::new(
            AdamWConfig::new()
                .with_cautious_weight_decay(true)
                .with_weight_decay(5e-5),
        )
    }
}

impl MnistTrainingConfig {
    pub fn small() -> Self {
        Self::new(
            AdamWConfig::new()
                .with_cautious_weight_decay(true)
                .with_weight_decay(5e-5),
        )
        .with_num_epochs(2)
        .with_batch_size(64)
        .with_num_workers(2)
    }
}

fn create_artifact_dir(artifact_dir: &str) {
    // Remove existing artifacts before to get an accurate learner summary
    std::fs::remove_dir_all(artifact_dir).ok();
    std::fs::create_dir_all(artifact_dir).ok();
}

pub fn run(
    experiment: &ExperimentRun,
    config: MnistTrainingConfig,
    devices: Vec<Device>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    experiment.log_args(&config)?;

    let device = devices.first().expect("No devices available").clone();
    device.seed(config.seed);

    let model = MnistModel::new(&device);

    // Training phase
    let result = train(model, &config, experiment);

    // Evaluation phase
    let dataset_test_plain = Arc::new(MnistDataset::test());
    let mut renderer = result.renderer;

    let idents_tests = generate_idents(None);

    for (ident, _) in idents_tests {
        let name = ident.to_string();
        renderer = evaluate(
            name.as_str(),
            ident,
            result.model.clone(),
            renderer,
            dataset_test_plain.clone(),
            config.batch_size,
        );
    }

    renderer.manual_close();

    // Save the artifact to the experiment
    let artifact = MnistModelArtifact {
        model_record: result.model.into_record(),
        config,
    };
    experiment.save_artifact("model", ArtifactKind::Model, artifact, &())?;

    Ok(())
}

fn train(
    model: MnistModel,
    config: &MnistTrainingConfig,
    experiment: &ExperimentRun,
) -> LearningResult<MnistModel> {
    create_artifact_dir(ARTIFACT_DIR);

    let dataset_train_original = Arc::new(MnistDataset::train());
    let dataset_train_plain = PartialDataset::new(dataset_train_original.clone(), 0, 10_000);
    let dataset_valid_plain = PartialDataset::new(dataset_train_original.clone(), 10_000, 12_000);

    let ident_trains = generate_idents(Some(1000));
    let ident_valid = generate_idents(Some(250));
    let dataset_train = DatasetIdent::compose(ident_trains, dataset_train_plain);
    let dataset_valid = DatasetIdent::compose(ident_valid, dataset_valid_plain);

    let dataloader_train = DataLoaderBuilder::new(MnistBatcher::default())
        .batch_size(config.batch_size)
        .shuffle(config.seed)
        .num_workers(config.num_workers)
        .build(dataset_train);
    let dataloader_valid = DataLoaderBuilder::new(MnistBatcher::default())
        .batch_size(config.batch_size)
        .shuffle(config.seed)
        .num_workers(config.num_workers)
        .build(dataset_valid);
    let lr_scheduler = ComposedLrSchedulerConfig::new()
        .cosine(CosineAnnealingLrSchedulerConfig::new(1.0, 2000))
        // Warmup
        .linear(LinearLrSchedulerConfig::new(1e-8, 1.0, 2000))
        .linear(LinearLrSchedulerConfig::new(1e-2, 1e-6, 10000));

    let mut training = SupervisedTraining::new(ARTIFACT_DIR, dataloader_train, dataloader_valid)
        .metrics((AccuracyMetric::new(), LossMetric::new()))
        .metric_train_numeric(LearningRateMetric::new())
        .early_stopping(MetricEarlyStoppingStrategy::new(
            &LossMetric::new(),
            Aggregate::Mean,
            Direction::Lowest,
            Split::Valid,
            StoppingCondition::NoImprovementSince { n_epochs: 5 },
        ))
        .num_epochs(config.num_epochs)
        .summary();

    let (model_ckpt, optim_ckpt, scheduler_ckpt) = match &config.resume_from {
        Some(resume) => {
            training = training.checkpoint(resume.epoch);
            experiment.checkpointers_from(ExperimentId::from(resume.experiment))
        }
        None => experiment.checkpointers(),
    };
    training = training
        .with_custom_checkpointers(model_ckpt, optim_ckpt, scheduler_ckpt)
        .with_metric_logger(experiment.metric_logger())
        .with_interrupter(experiment.interrupter());

    training.launch(Learner::new(
        model,
        config.optimizer.init(),
        lr_scheduler.init().unwrap(),
    ))
}

fn evaluate(
    name: &str,
    ident: DatasetIdent,
    model: MnistModel,
    renderer: Box<dyn MetricsRenderer>,
    dataset: impl Dataset<MnistItem> + 'static,
    batch_size: usize,
) -> Box<dyn MetricsRenderer> {
    let batcher = MnistBatcher::default();
    let dataset_test = DatasetIdent::prepare(ident, dataset);
    let dataloader_test = DataLoaderBuilder::new(batcher)
        .batch_size(batch_size)
        .num_workers(2)
        .build(dataset_test);

    let evaluator = EvaluatorBuilder::new(ARTIFACT_DIR)
        .renderer(renderer)
        .metrics((AccuracyMetric::new(), LossMetric::new()))
        .build(model);

    evaluator.eval(name, dataloader_test)
}

enum DatasetIdent {
    Plain,
    Transformed(Vec<Transform>),
    All,
}

impl core::fmt::Display for DatasetIdent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatasetIdent::Plain => f.write_str("Plain")?,
            DatasetIdent::Transformed(items) => {
                for i in 0..items.len() {
                    f.write_fmt(format_args!("{}", items[i]))?;
                    if i < items.len() - 1 {
                        f.write_str(" ")?;
                    }
                }
            }
            DatasetIdent::All => f.write_str("All")?,
        };

        Ok(())
    }
}

impl DatasetIdent {
    pub fn many(transforms: Vec<Transform>) -> Self {
        Self::Transformed(transforms)
    }

    pub fn prepare(self, dataset: impl Dataset<MnistItem>) -> impl Dataset<MnistItemPrepared> {
        let items = match self {
            DatasetIdent::Plain => Vec::new(),
            DatasetIdent::All => {
                vec![
                    Transform::Translate,
                    Transform::Shear,
                    Transform::Scale,
                    Transform::Rotation,
                ]
            }
            DatasetIdent::Transformed(items) => items.clone(),
        };
        MapperDataset::new(dataset, MnistMapper::default().transform(&items))
    }

    pub fn compose(
        idents: Vec<(Self, Option<usize>)>,
        dataset: PartialDataset<Arc<MnistDataset>, MnistItem>,
    ) -> impl Dataset<MnistItemPrepared> {
        let datasets = idents
            .into_iter()
            .map(|(ident, size)| match size {
                Some(size) => {
                    SamplerDataset::with_replacement(ident.prepare(dataset.clone()), size)
                }
                None => {
                    let dataset = ident.prepare(dataset.clone());
                    let size = dataset.len();
                    SamplerDataset::without_replacement(dataset, size)
                }
            })
            .collect();
        ComposedDataset::new(datasets)
    }
}

fn generate_idents(num_samples_base: Option<usize>) -> Vec<(DatasetIdent, Option<usize>)> {
    let mut current = Vec::new();
    let mut idents = Vec::new();

    for shear in [None, Some(Transform::Shear)] {
        for scale in [None, Some(Transform::Scale)] {
            for rotation in [None, Some(Transform::Rotation)] {
                for translate in [None, Some(Transform::Translate)] {
                    if let Some(tr) = shear {
                        current.push(tr);
                    }
                    if let Some(tr) = scale {
                        current.push(tr);
                    }
                    if let Some(tr) = rotation {
                        current.push(tr);
                    }
                    if let Some(tr) = translate {
                        current.push(tr);
                    }

                    let num_samples = num_samples_base.map(|val| val * current.len());

                    if current.len() == 4 {
                        idents.push((DatasetIdent::All, num_samples));
                    } else if current.is_empty() {
                        idents.push((DatasetIdent::Plain, num_samples));
                    } else {
                        idents.push((DatasetIdent::many(current.clone()), num_samples));
                    }

                    current.clear();
                }
            }
        }
    }

    idents
}
