use crate::parse::common;
use rayon::iter::{IntoParallelRefIterator, IntoParallelRefMutIterator, ParallelIterator};
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::Deref,
    path::PathBuf,
};

pub mod csharp;
pub mod yaml;
pub mod json;
pub mod fluent;
