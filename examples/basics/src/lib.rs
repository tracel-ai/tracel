//! Toy capabilities used by the `basics` examples, shaped like real ones so they double as
//! templates:
//!
//! - [`WordTokenizer`] — a streaming [`Inference`](tracel::inference::Inference) that splits each
//!   prompt into whitespace tokens and streams them back one at a time.
//! - [`training`] — a stand-in experiment that tracks nested activities, logs metrics, and honors
//!   cancellation, exactly as a real training loop would (only the per-step math is fake).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracel::inference::{Inference, InferenceInput, InferenceOutput, InferenceSession};

/// One prompt fed to the [`WordTokenizer`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Prompt {
    pub text: String,
}

/// One streamed output token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token: String,
}

/// A streaming inference shaped like a real one: model state lives in `&self`, and `infer` pulls
/// inputs and writes outputs as it goes. Here the "model" is just whitespace splitting.
pub struct WordTokenizer {
    per_token_delay: Duration,
}

impl WordTokenizer {
    pub fn new() -> Self {
        Self {
            per_token_delay: Duration::ZERO,
        }
    }

    /// Pauses `delay` before each token, to make streaming observable.
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

            // Explicit metric through the session. Discarded offline; shipped with Cloud.
            session
                .with_attributes([("prompt_len", prompt.text.len() as u64)])
                .log_gauge("prompt_tokens", words.len() as f64);

            // Routed to the session by the tracing layer when one is installed.
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
                    // The consumer disconnected; stop early.
                    return;
                }
                emitted += 1;
            }

            session.log_counter("tokens_emitted", emitted);
        }
    }
}

/// A stand-in training experiment, shaped like a real one.
pub mod training {
    use serde::{Deserialize, Serialize};
    use tracel::experiment::{ExperimentRun, MetricSpec, MetricValue};

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    /// Config for the toy run. A real one would carry model hyperparameters, dataset paths, etc.
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

    /// Run the toy training loop.
    ///
    /// It declares its metrics up front, tracks nested activities (`training` -> per-epoch) with
    /// progress and cancellation, and logs per-batch metrics plus per-epoch summaries. Replace the
    /// `sleep` with a real forward/backward pass and the surrounding experiment plumbing is
    /// unchanged. See the `mnist` example for the same tracking wired automatically through the
    /// Burn `train` integration.
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

                // Stand in for a real training step.
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
