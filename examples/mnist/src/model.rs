use crate::{data::MnistBatch, training::MnistTrainingConfig};
use burn::{
    nn::{
        BatchNorm, PaddingConfig2d,
        loss::CrossEntropyLossConfig,
        pool::{MaxPool2d, MaxPool2dConfig},
    },
    prelude::*,
    store::ModuleRecord,
    tensor::Bytes,
    train::{ClassificationOutput, InferenceStep, TrainOutput, TrainStep},
};
use tracel::artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

#[derive(Module, Debug)]
pub struct MnistModel {
    conv1: ConvBlock,
    conv2: ConvBlock,
    dropout: nn::Dropout,
    fc1: nn::Linear,
    fc2: nn::Linear,
    fc3: nn::Linear,
    activation: nn::Gelu,
}

impl Default for MnistModel {
    fn default() -> Self {
        let device = Device::default();
        Self::new(&device)
    }
}

const NUM_CLASSES: usize = 10;

impl MnistModel {
    pub fn new(device: &Device) -> Self {
        let conv1 = ConvBlock::new([1, 64], [3, 3], device, true); // out: max_pool -> [Batch,32,13,13]
        let conv2 = ConvBlock::new([64, 64], [3, 3], device, true); // out: max_pool -> [Batch,64,5,5]
        let hidden_size = 64 * 5 * 5;
        let fc1 = nn::LinearConfig::new(hidden_size, 128).init(device);
        let fc2 = nn::LinearConfig::new(128, 128).init(device);
        let fc3 = nn::LinearConfig::new(128, NUM_CLASSES).init(device);

        let dropout = nn::DropoutConfig::new(0.25).init();

        Self {
            conv1,
            conv2,
            dropout,
            fc1,
            fc2,
            fc3,
            activation: nn::Gelu::new(),
        }
    }

    pub fn forward(&self, input: Tensor<3>) -> Tensor<2> {
        let [batch_size, height, width] = input.dims();

        let x = input.reshape([batch_size, 1, height, width]).detach();
        let x = self.conv1.forward(x);
        let x = self.conv2.forward(x);

        let [batch_size, channels, height, width] = x.dims();
        let x = x.reshape([batch_size, channels * height * width]);

        let x = self.fc1.forward(x);
        let x = self.activation.forward(x);
        let x = self.dropout.forward(x);

        let x = self.fc2.forward(x);
        let x = self.activation.forward(x);
        let x = self.dropout.forward(x);

        self.fc3.forward(x)
    }

    pub fn forward_classification(&self, item: MnistBatch) -> ClassificationOutput {
        let targets = item.targets;
        let output = self.forward(item.images);
        let loss = CrossEntropyLossConfig::new()
            .init(&output.device())
            .forward(output.clone(), targets.clone());

        ClassificationOutput {
            loss,
            output,
            targets,
        }
    }
}

#[derive(Module, Debug)]
pub struct ConvBlock {
    conv: nn::conv::Conv2d,
    norm: BatchNorm,
    pool: Option<MaxPool2d>,
    activation: nn::Relu,
}

impl ConvBlock {
    pub fn new(channels: [usize; 2], kernel_size: [usize; 2], device: &Device, pool: bool) -> Self {
        let conv = nn::conv::Conv2dConfig::new(channels, kernel_size)
            .with_padding(PaddingConfig2d::Valid)
            .init(device);
        let norm = nn::BatchNormConfig::new(channels[1]).init(device);
        let pool = if pool {
            Some(MaxPool2dConfig::new([2, 2]).with_strides([2, 2]).init())
        } else {
            None
        };

        Self {
            conv,
            norm,
            pool,
            activation: nn::Relu::new(),
        }
    }

    pub fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let x = self.conv.forward(input);
        let x = self.norm.forward(x);
        let x = self.activation.forward(x);

        if let Some(pool) = &self.pool {
            pool.forward(x)
        } else {
            x
        }
    }
}

impl TrainStep for MnistModel {
    type Input = MnistBatch;
    type Output = ClassificationOutput;

    fn step(&self, item: MnistBatch) -> TrainOutput<ClassificationOutput> {
        let item = self.forward_classification(item);

        TrainOutput::new(self, item.loss.backward(), item)
    }
}

impl InferenceStep for MnistModel {
    type Input = MnistBatch;
    type Output = ClassificationOutput;

    fn step(&self, item: MnistBatch) -> ClassificationOutput {
        self.forward_classification(item)
    }
}

// Define the model artifact (put in that everything you will need for inference)
#[derive(Debug)]
pub struct MnistModelArtifact {
    pub model_record: ModuleRecord,
    pub config: MnistTrainingConfig,
}

impl BundleEncode for MnistModelArtifact {
    type Settings = ();
    type Error = String;

    fn encode<O: BundleSink>(
        self,
        sink: &mut O,
        _settings: &Self::Settings,
    ) -> Result<(), Self::Error> {
        let config_bytes = serde_json::to_vec(&self.config)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        sink.put_bytes("config.json", &config_bytes)
            .map_err(|e| format!("Failed to write config: {e}"))?;

        let model_bytes = self
            .model_record
            .into_bytes()
            .map_err(|e| format!("Failed to record model: {e}"))?;

        sink.put_bytes("model.mpk", &model_bytes)
            .map_err(|e| format!("Failed to write model: {e}"))?;

        Ok(())
    }
}

impl BundleDecode for MnistModelArtifact {
    type Settings = ();
    type Error = String;

    fn decode<I: BundleSource>(
        source: &I,
        _settings: &Self::Settings,
    ) -> Result<Self, Self::Error> {
        let config_reader = source
            .open("config.json")
            .map_err(|e| format!("Failed to read config: {e}"))?;
        let config: MnistTrainingConfig = serde_json::from_reader(config_reader)
            .map_err(|e| format!("Failed to deserialize config: {e}"))?;

        let mut model_reader = source
            .open("model.mpk")
            .map_err(|e| format!("Failed to read model: {e}"))?;
        let mut model_bytes = Vec::new();

        model_reader
            .read_to_end(&mut model_bytes)
            .map_err(|e| format!("Failed to read model: {e}"))?;

        let model_record = ModuleRecord::from_bytes(Bytes::from_bytes_vec(model_bytes))
            .map_err(|e| format!("Failed to read model: {e}"))?;

        Ok(MnistModelArtifact {
            model_record,
            config,
        })
    }
}
