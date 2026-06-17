use std::{error::Error, marker::PhantomData};

use clap::Parser;

use crate::mapper::Mapper;

pub struct ClapMapper<I> {
    _marker: PhantomData<I>,
}

impl<I> ClapMapper<I> {
    fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<I: Parser> Mapper<I> for ClapMapper<I> {
    fn map(&self, raw: &str) -> Result<I, Box<dyn Error + Send + Sync>> {
        let args = raw.split_whitespace();
        I::try_parse_from(std::iter::once("").chain(args)).map_err(Into::into)
    }
}
