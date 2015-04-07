#![feature(collections)]
#![feature(core)]
#![feature(custom_attribute)]
#![feature(old_path)]
#![feature(plugin)]
#![feature(convert)]
#![feature(std_misc)]
#![plugin(gfx_macros)]

extern crate collada;
extern crate gfx;
extern crate gfx_debug_draw;
extern crate gfx_device_gl;
extern crate gfx_texture;
extern crate quack;
extern crate quaternion;
extern crate vecmath;
extern crate interpolation;

// TODO - 'SkinnedRenderer' probably belongs in its own crate,
// then we wouldn't need the following dependencies here

pub mod animation;
pub mod skinned_renderer;
mod math;

pub use animation::{
    AnimationClip,
    AnimationSample,
    calculate_global_poses,
    SQT,
    draw_skeleton,
    BlendTreeNode,
    ClipNode,
    LerpNode,
};
pub use skinned_renderer::SkinnedRenderer;