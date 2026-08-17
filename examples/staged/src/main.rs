#![recursion_limit = "256"]

use std::sync::Arc;

use burn::{
    data::{
        dataloader::{DataLoader, DataLoaderBuilder, batcher::Batcher},
        dataset::InMemDataset,
    },
    lr_scheduler::constant::ConstantLr,
    module::Param,
    optim::SgdConfig,
    prelude::*,
    tensor::Distribution,
    train::{
        InferenceStep, Learner, RegressionOutput, SupervisedTraining, TrainOutput, TrainStep,
        metric::LossMetric,
    },
};
use tracel::experiment::{
    ExperimentRun, MetricValue, integration::training::SupervisedTrainingExperimentExt,
};

fn main() -> anyhow::Result<()> {
    let module = common::context()?.experiment();

    module
        .create("staged-training", |run: &ExperimentRun, ()| study(run))
        .run(())
        .map_err(|error| anyhow::anyhow!("staged training failed: {error}"))?;

    Ok(())
}

fn study(run: &ExperimentRun) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let fold_count = 3usize;
    run.activity("Prepare data")
        .meter(fold_count as u64, "folds")
        .run(|preparation| -> anyhow::Result<()> {
            for fold in 0..fold_count {
                preparation.message(format!("Prepared fold {}", fold + 1));
                preparation.inc(1);
            }
            Ok(())
        })?;

    run.activity("Cross-validation")
        .run(|folds| -> anyhow::Result<()> {
            let mut scores = Vec::with_capacity(fold_count);

            for fold_index in 0..fold_count {
                let fold = folds
                    .activity(format!("Fold {}", fold_index + 1))
                    .cancellable()
                    .start();
                fold.log_info("Launching toy learner");

                let device = Device::flex().autodiff();
                let model = ToyModel::new(&device);
                let optimizer = SgdConfig::new().init();
                let scheduler = ConstantLr::new(1e-3);
                let (train, valid) = dataloaders();
                let artifacts = tempfile::tempdir()?;

                SupervisedTraining::new(artifacts.path(), train, valid)
                    .num_epochs(1)
                    .metric_train_numeric(LossMetric::new())
                    .metric_valid_numeric(LossMetric::new())
                    .with_experiment(&fold)
                    .launch(Learner::new(model, optimizer, scheduler));

                if fold.is_cancel_requested() {
                    fold.abandon_with_message("Training cancelled");
                    continue;
                }
                scores.push(1.0 / (fold_index + 1) as f64);
                fold.finish();
            }

            let mean_score = if scores.is_empty() {
                0.0
            } else {
                scores.iter().sum::<f64>() / scores.len() as f64
            };
            folds.log_summary(vec![MetricValue {
                name: "mean_fold_score".to_string(),
                value: mean_score,
            }]);

            Ok(())
        })?;

    Ok(())
}

#[derive(Module, Debug)]
struct ToyModel {
    weight: Param<Tensor<2>>,
}

impl ToyModel {
    fn new(device: &Device) -> Self {
        Self {
            weight: Param::from_tensor(Tensor::random([1, 2], Distribution::Default, device)),
        }
    }
}

#[derive(Clone, Debug)]
struct ToyBatch {
    target: Tensor<2>,
}

struct ToyBatcher;

impl Batcher<(), ToyBatch> for ToyBatcher {
    fn batch(&self, _items: Vec<()>, device: &Device) -> ToyBatch {
        ToyBatch {
            target: Tensor::zeros([1, 2], device),
        }
    }
}

impl TrainStep for ToyModel {
    type Input = ToyBatch;
    type Output = RegressionOutput;

    fn step(&self, batch: ToyBatch) -> TrainOutput<RegressionOutput> {
        let output = self.weight.val();
        let loss = output
            .clone()
            .sub(batch.target.clone())
            .powi_scalar(2)
            .mean();
        let regression = RegressionOutput::new(loss.clone(), output, batch.target);
        TrainOutput::new(self, loss.backward(), regression)
    }
}

impl InferenceStep for ToyModel {
    type Input = ToyBatch;
    type Output = RegressionOutput;

    fn step(&self, batch: ToyBatch) -> RegressionOutput {
        let output = self.weight.val();
        let loss = output
            .clone()
            .sub(batch.target.clone())
            .powi_scalar(2)
            .mean();
        RegressionOutput::new(loss, output, batch.target)
    }
}

fn dataloaders() -> (Arc<dyn DataLoader<ToyBatch>>, Arc<dyn DataLoader<ToyBatch>>) {
    let train = DataLoaderBuilder::new(ToyBatcher)
        .batch_size(2)
        .build(InMemDataset::new(vec![(); 4]));
    let valid = DataLoaderBuilder::new(ToyBatcher)
        .batch_size(2)
        .build(InMemDataset::new(vec![(); 2]));
    (train, valid)
}
