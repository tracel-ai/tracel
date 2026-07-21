//! Toy inference and experiment capabilities used by the `basics` examples.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracel::inference::{Inference, InferenceInput, InferenceOutput, InferenceSession};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Prompt {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token: String,
}

/// Splits each prompt into whitespace tokens and streams them back one at a time.
pub struct WordTokenizer {
    per_token_delay: Duration,
}

impl WordTokenizer {
    pub fn new() -> Self {
        Self {
            per_token_delay: Duration::ZERO,
        }
    }

    /// Pause before each token so streaming is observable.
    pub fn with_delay(delay: Duration) -> Self {
        Self {
            per_token_delay: delay,
        }
    }
}

impl Default for WordTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Inference for WordTokenizer {
    type Input = Prompt;
    type Output = Token;

    fn infer(
        &self,
        session: &InferenceSession,
        input: InferenceInput<Prompt>,
        output: InferenceOutput<Token>,
    ) {
        for prompt in input {
            let words: Vec<&str> = prompt.text.split_whitespace().collect();

            session
                .with_attributes([("prompt_len", prompt.text.len() as u64)])
                .log_gauge("prompt_tokens", words.len() as f64);
            tracing::info!(tokens = words.len(), "tokenizing prompt");

            let mut emitted: u64 = 0;
            for word in words {
                if !self.per_token_delay.is_zero() {
                    std::thread::sleep(self.per_token_delay);
                }
                if output
                    .write(Token {
                        token: word.to_string(),
                    })
                    .is_err()
                {
                    return; // consumer disconnected
                }
                emitted += 1;
            }

            session.log_counter("tokens_emitted", emitted);
        }
    }
}

/// A stand-in training loop with the plumbing a real one uses: metric definitions, nested activity
/// tracking, per-batch metrics, progress logs, and cancellation. Only the per-step math is fake.
pub mod training {
    use serde::{Deserialize, Serialize};
    use tracel::experiment::{ExperimentRun, MetricSpec, MetricValue};

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TrainingConfig {
        pub epochs: usize,
        pub batches_per_epoch: usize,
    }

    impl Default for TrainingConfig {
        fn default() -> Self {
            Self {
                epochs: 3,
                batches_per_epoch: 8,
            }
        }
    }

    pub fn train(experiment: &ExperimentRun, config: TrainingConfig) -> Result<(), BoxError> {
        experiment.log_args(&config)?;
        experiment.log_metric_definition(MetricSpec {
            name: "loss".to_string(),
            description: Some("training loss".to_string()),
            unit: None,
            higher_is_better: false,
        })?;
        experiment.log_metric_definition(MetricSpec {
            name: "accuracy".to_string(),
            description: Some("training accuracy".to_string()),
            unit: Some("ratio".to_string()),
            higher_is_better: true,
        })?;

        let total_steps = (config.epochs * config.batches_per_epoch) as f64;

        let mut run = experiment
            .activity("training")
            .progress()
            .total(config.epochs as u64)
            .unit("epoch")
            .cancellable()
            .start();

        for epoch in 1..=config.epochs {
            let mut epoch_activity = run
                .activity(format!("epoch {epoch}"))
                .progress()
                .total(config.batches_per_epoch as u64)
                .unit("batch")
                .cancellable()
                .attr("epoch", epoch)?
                .start();

            let mut loss_sum = 0.0;
            let mut accuracy_sum = 0.0;

            for batch in 1..=config.batches_per_epoch {
                if epoch_activity.is_cancel_requested() {
                    epoch_activity.abandon_with_message("cancel requested");
                    run.abandon_with_message("cancel requested");
                    return Ok(());
                }

                std::thread::sleep(std::time::Duration::from_millis(200));

                let step = ((epoch - 1) * config.batches_per_epoch + batch) as f64;
                let loss = 1.2 / (1.0 + step * 0.3);
                let accuracy = (0.5 + step / total_steps * 0.45).min(0.97);

                experiment.log_metric(
                    epoch,
                    "train",
                    batch,
                    vec![
                        MetricValue {
                            name: "loss".to_string(),
                            value: loss,
                        },
                        MetricValue {
                            name: "accuracy".to_string(),
                            value: accuracy,
                        },
                    ],
                )?;

                loss_sum += loss;
                accuracy_sum += accuracy;
                epoch_activity.inc(1);

                experiment.log_info(format!(
                    "epoch {epoch}/{} · batch {batch}/{} · {:.0}% complete · loss={loss:.3}",
                    config.epochs,
                    config.batches_per_epoch,
                    step / total_steps * 100.0,
                ))?;
            }

            let mean_loss = loss_sum / config.batches_per_epoch as f64;
            let mean_accuracy = accuracy_sum / config.batches_per_epoch as f64;
            experiment.log_epoch_summary(
                epoch,
                "train",
                vec![
                    MetricValue {
                        name: "loss".to_string(),
                        value: mean_loss,
                    },
                    MetricValue {
                        name: "accuracy".to_string(),
                        value: mean_accuracy,
                    },
                ],
            )?;
            epoch_activity
                .finish_with_message(format!("loss={mean_loss:.3} acc={mean_accuracy:.3}"));
            run.inc(1);
        }

        run.finish_with_message("training complete");
        Ok(())
    }
}
