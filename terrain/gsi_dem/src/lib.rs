pub use yamamuki_terrain::{
    natural_codec, natural_local_gradient, natural_regional_surface, natural_surface,
};
pub mod natural_inventory;
pub mod natural_shared_pipeline;
pub mod natural_shared_resources;
pub mod natural_shared_store;

pub mod local_slope;

pub mod local_slope_prune;

pub mod automatic_slope;

pub mod slope_smoothing;

pub mod national_smoothing;
