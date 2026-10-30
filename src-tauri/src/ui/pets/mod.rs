//! Native pet engine (wave 1 of the WebView -> native rewrite): every pet of src/pets*.js and the capsule characters of src/characters.js,
//! drawn with Direct2D from data exported out of the web version (tools/dev/export-pets.mjs). Consumers arrive with the shell wave.
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
