/// The input handed to [`Inference::infer`](crate::Inference::infer): a stream of already-decoded,
/// typed items.
///
/// Iterate it to consume the stream. Transport and decode failures are handled at the transport
/// boundary before items reach here, so pulling an item never fails; a completed stream simply
/// ends. For the single-item case use [`once`](Self::once).
pub struct InferenceInput<I> {
    items: Box<dyn Iterator<Item = I> + Send>,
}

impl<I> InferenceInput<I> {
    /// Wrap an iterator of already-decoded items as an input stream.
    pub(crate) fn from_items<It>(items: It) -> Self
    where
        It: Iterator<Item = I> + Send + 'static,
    {
        Self {
            items: Box::new(items),
        }
    }

    /// Build an input that yields a single item and then ends.
    pub fn once(input: I) -> Self
    where
        I: Send + 'static,
    {
        Self::from_items(std::iter::once(input))
    }
}

impl<I> Iterator for InferenceInput<I> {
    type Item = I;

    fn next(&mut self) -> Option<I> {
        self.items.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_yields_single_item_then_ends() {
        let mut input = InferenceInput::once(42);
        assert_eq!(input.next(), Some(42));
        assert_eq!(input.next(), None);
    }

    #[test]
    fn from_items_drains_in_order() {
        let input = InferenceInput::from_items(vec![1, 2, 3].into_iter());
        let collected: Vec<i32> = input.collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }
}
