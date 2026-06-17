use std::error::Error;

mod clap_mapper;
mod json_mapper;
mod preset_mapper;

pub use clap_mapper::ClapMapper;
pub use json_mapper::JsonMapper;
pub use preset_mapper::PresetMapper;

pub trait Mapper<I> {
    fn map(&self, raw: &str) -> Result<I, Box<dyn Error + Send + Sync>>;
}
