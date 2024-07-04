use crate::{data::MnistBatcher, model::Model};
use burn::{
    data::{
        dataloader::batcher::Batcher,
        dataset::{vision::MnistItem, Dataset},
    },
    prelude::*,
    tensor::backend::AutodiffBackend,
};
use tracel::heat::macros::heat;

pub fn infer<B: Backend>(model: Model<B>, device: B::Device, item: MnistItem) {
    let label = item.label;
    let batcher = MnistBatcher::new(device);
    let batch = batcher.batch(vec![item]);
    let output = model.forward(batch.images);
    let predicted = output.argmax(1).flatten::<1>(0, 1).into_scalar();

    println!("Predicted {} Expected {}", predicted, label);
}

#[heat(inference)]
pub(crate) fn inference<B: AutodiffBackend>(model: Model<B>, device: B::Device) -> () {
    crate::inference::infer::<B>(
        model,
        device,
        burn::data::dataset::vision::MnistDataset::test()
            .get(42)
            .unwrap(),
    );
}
