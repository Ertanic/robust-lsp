#![allow(dead_code)]

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct Size2d {
    x: u32,
    y: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RsiState {
    pub name: String,
    pub directions: Option<u8>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct RsiMeta {
    pub version: u32,
    pub license: String,
    pub copyright: String,
    pub size: Size2d,
    pub states: Vec<RsiState>,
}
