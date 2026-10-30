//! Native pet engine: every pet and the capsule characters, drawn with Direct2D from frozen data (`data/*.json`, one file per pet).
#![allow(dead_code)]

pub mod characters;
pub mod data;
pub mod ease;
pub mod engine;
pub mod hooks;
pub mod interp;
pub mod model;
pub mod path;
pub mod rng;
pub mod sim;
pub mod render;

#[cfg(test)]
mod tests;
