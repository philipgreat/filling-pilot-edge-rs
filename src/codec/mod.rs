//! PLC Data Block Codec Module

mod codec;
mod data_block;
mod property;

pub use codec::{Codec, CodecRegistry};
pub use data_block::{DataBlock, DataBlockDefinition, PropertyType};
pub use property::DataBlockProperty;
