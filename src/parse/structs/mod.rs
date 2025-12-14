use crate::parse::common;
use rayon::iter::{IntoParallelRefIterator, IntoParallelRefMutIterator, ParallelIterator};
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::Deref,
    path::PathBuf,
};

pub mod csharp;
pub mod fluent;
pub mod json;
pub mod yaml;
