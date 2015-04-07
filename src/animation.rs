use collada::Animation as ColladaAnim;
use collada::Skeleton;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::num::Float;
use vecmath::{Vector3, Matrix4, mat4_id, row_mat4_transform, row_mat4_mul, mat4_transposed};
use quaternion::id as quaternion_id;
use quaternion::Quaternion;

use gfx::{Device};
use gfx_debug_draw::DebugRenderer;

use gfx_device_gl::Device as GlDevice;
use gfx_device_gl::Factory as GlFactory;

use math::{quaternion_to_matrix, matrix_to_quaternion};

use interpolation::{Spatial, lerp};

pub trait BlendTreeNode {
    fn get_output_pose(&self, elapsed_time: f32, output_poses: &mut [SQT]);
}

pub struct LerpNode<'a> {
    pub blend_parameter: f32,
    pub inputs: [&'a BlendTreeNode; 2],
}

impl<'a> BlendTreeNode for LerpNode<'a> {
    fn get_output_pose(&self, elapsed_time: f32, output_poses: &mut [SQT]) {

        let mut input_poses = [ SQT { translation: [0.0, 0.0, 0.0], scale: 0.0, rotation: (0.0, [0.0, 0.0, 0.0]) }; 64 ];

        let sample_count = output_poses.len();

        self.inputs[0].get_output_pose(elapsed_time, &mut input_poses[0 .. sample_count]);
        self.inputs[1].get_output_pose(elapsed_time, output_poses);

        for i in (0 .. output_poses.len()) {
            let pose_1 = input_poses[i];
            let pose_2 = &mut output_poses[i];
            pose_2.scale = lerp(&pose_1.scale, &pose_2.scale, &self.blend_parameter);
            pose_2.translation = lerp(&pose_1.translation, &pose_2.translation, &self.blend_parameter);
            pose_2.rotation = lerp_quaternion(&pose_1.rotation, &pose_2.rotation, &self.blend_parameter);
        }

    }
}

pub struct ClipNode<'a> {
    pub start_time: f32,
    pub clip: &'a AnimationClip,
}

impl<'a> BlendTreeNode for ClipNode<'a> {
    fn get_output_pose(&self, elapsed_time: f32, output_poses: &mut [SQT]) {
        self.clip.get_interpolated_poses_at_time(elapsed_time, output_poses);
    }
}

#[derive(Debug)]
pub struct AnimationClip {
    pub samples: Vec<AnimationSample>,

    ///
    /// Assumes constant sample rate for animation
    ///
    pub samples_per_second: f32,
}

fn lerp_quaternion(q1: &Quaternion<f32>, q2: &Quaternion<f32>, blend_factor: &f32) -> Quaternion<f32> {

    // interpolate

    let blend_factor_recip = 1.0 - blend_factor;
    let w = blend_factor_recip * q1.0 + blend_factor * q2.0;
    let x = blend_factor_recip * q1.1[0] + blend_factor * q2.1[0];
    let y = blend_factor_recip * q1.1[1] + blend_factor * q2.1[1];
    let z = blend_factor_recip * q1.1[2] + blend_factor * q2.1[2];

    // renormalize

    let len = (w * w + x * x + y * y + z * z).sqrt();
    (w/len, [x / len, y / len, z /len])
}

impl AnimationClip {

    pub fn sample_at_time(&self, elapsed_time: f32) -> &AnimationSample {
        let sample_index = (elapsed_time * self.samples_per_second) as usize % self.samples.len();
        &self.samples[sample_index]
    }

    ///
    /// Sets sample_per_second such that the animation will have the given
    /// duration
    ///
    pub fn set_duration(&mut self, duration: f32) {
        self.samples_per_second = self.samples.len() as f32 / duration;
    }

    pub fn get_interpolated_poses_at_time(&self, elapsed_time: f32, blended_poses: &mut [SQT]) {

        let interpolated_index = elapsed_time * self.samples_per_second;

        let index_1 = interpolated_index.floor() as usize;
        let index_2 = interpolated_index.ceil() as usize;

        let blend_factor = interpolated_index - index_1 as f32;

        let index_1 = index_1 % self.samples.len();
        let index_2 = index_2 % self.samples.len();

        let sample_1 = &self.samples[index_1];
        let sample_2 = &self.samples[index_2];


        for i in (0 .. sample_1.local_poses.len()) {

            let pose_1 = &sample_1.local_poses[i];
            let pose_2 = &sample_2.local_poses[i];

            let blended_pose = &mut blended_poses[i];
            blended_pose.scale = lerp(&pose_1.scale, &pose_2.scale, &blend_factor);
            blended_pose.translation = lerp(&pose_1.translation, &pose_2.translation, &blend_factor);
            blended_pose.rotation = lerp_quaternion(&pose_1.rotation, &pose_2.rotation, &blend_factor);

        }

    }


    pub fn from_collada(skeleton: &Skeleton, animations: &Vec<ColladaAnim>) -> AnimationClip {
        use std::f32::consts::PI;

        // Z-axis is 'up' in COLLADA, so need to rotate root pose about x-axis so y-axis is 'up'
        let rotate_on_x =
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, (PI/2.0).cos(), (PI/2.0).sin(), 0.0],
                [0.0, (-PI/2.0).sin(), (PI/2.0).cos(), 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ];

        // Build an index of joint names to anims
        let mut joint_animations = HashMap::new();
        for anim in animations.iter() {
            let joint_name = anim.target.split('/').next().unwrap();
            joint_animations.insert(joint_name, anim);
        }

        // Assuming all ColladaAnims have the same number of samples..
        let sample_count = animations[0].sample_times.len();

        // Assuming all ColladaAnims have the same duration..
        let duration = *animations[0].sample_times.last().unwrap();

        // Assuming constant sample rate
        let samples_per_second = sample_count as f32 / duration;

        let samples = (0 .. sample_count).map(|sample_index| {

            // Grab local poses for each joint from COLLADA animation if available,
            // falling back to identity matrix
            let local_poses: Vec<Matrix4<f32>> = skeleton.joints.iter().map(|joint| {
                match joint_animations.get(&joint.name[..]) {
                    Some(a) if joint.is_root() => row_mat4_mul(rotate_on_x, a.sample_poses[sample_index]),
                    Some(a) => a.sample_poses[sample_index], // convert col major to row major
                    None => mat4_id(),
                }
            }).collect();

            // Convert local poses to SQT (for interpolation)
            let local_poses: Vec<SQT> = local_poses.iter().map(|pose_matrix| {
                SQT {
                    translation: [
                        pose_matrix[0][3],
                        pose_matrix[1][3],
                        pose_matrix[2][3],
                    ],
                    scale: 1.0, // TODO don't assume?
                    rotation: matrix_to_quaternion(pose_matrix),
                }
            }).collect();

            AnimationSample {
                local_poses: local_poses,
            }
        }).collect();

        AnimationClip {
            samples_per_second: samples_per_second,
            samples: samples,
        }
    }
}

///
/// FIXME - don't allocate a new Vec!
///
pub fn calculate_global_poses(
    skeleton: &Skeleton,
    local_poses: &[SQT],
) -> Vec<Matrix4<f32>> {

    let mut global_poses: Vec<Matrix4<f32>> = Vec::new();

    for (joint_index, joint) in skeleton.joints.iter().enumerate() {

        let parent_pose = if !joint.is_root() {
            global_poses[joint.parent_index as usize]
        } else {
            mat4_id()
        };

        let local_pose_sqt = &local_poses[joint_index];

        let mut local_pose = quaternion_to_matrix(local_pose_sqt.rotation);

        local_pose[0][3] = local_pose_sqt.translation[0];
        local_pose[1][3] = local_pose_sqt.translation[1];
        local_pose[2][3] = local_pose_sqt.translation[2];

        global_poses.push(row_mat4_mul(parent_pose, local_pose));
    }

    global_poses
}

pub fn draw_skeleton(skeleton: &Skeleton, global_poses: &Vec<Matrix4<f32>>, debug_renderer: &mut DebugRenderer<GlDevice, GlFactory>, draw_labels: bool) {
    for (joint_index, joint) in skeleton.joints.iter().enumerate() {

        let joint_position = row_mat4_transform(global_poses[joint_index], [0.0, 0.0, 0.0, 1.0]);

        let leaf_end = row_mat4_transform(
            global_poses[joint_index],
            [0.0, 1.0, 0.0, 1.0]
        );

        if !joint.is_root() {
            let parent_position = row_mat4_transform(global_poses[joint.parent_index as usize], [0.0, 0.0, 0.0, 1.0]);

            // Draw bone (between joint and parent joint)

            debug_renderer.draw_line(
                [parent_position[0], parent_position[1], parent_position[2]],
                [joint_position[0], joint_position[1], joint_position[2]],
                [0.2, 0.2, 0.2, 1.0]
            );

            if !skeleton.joints.iter().any(|j| j.parent_index as usize == joint_index) {

                // Draw extension along joint's y-axis...
                debug_renderer.draw_line(
                    [joint_position[0], joint_position[1], joint_position[2]],
                    [leaf_end[0], leaf_end[1], leaf_end[2]],
                    [0.2, 0.2, 0.2, 1.0]
                );
            }
        }

        if draw_labels {
            // Label joint
            debug_renderer.draw_text_at_position(
                &joint.name[..],
                [leaf_end[0], leaf_end[1], leaf_end[2]],
                [1.0, 1.0, 1.0, 1.0]
            );
        }

        // Draw joint-relative axes
        let p_x_axis = row_mat4_transform(
            global_poses[joint_index],
            [1.0, 0.0, 0.0, 1.0]
        );

        let p_y_axis = row_mat4_transform(
            global_poses[joint_index],
            [0.0, 1.0, 0.0, 1.0]
        );

        let p_z_axis = row_mat4_transform(
            global_poses[joint_index],
            [0.0, 0.0, 1.0, 1.0]
        );

        debug_renderer.draw_line(
            [joint_position[0], joint_position[1], joint_position[2]],
            [p_x_axis[0], p_x_axis[1], p_x_axis[2]],
            [1.0, 0.2, 0.2, 1.0]
        );

        debug_renderer.draw_line(
            [joint_position[0], joint_position[1], joint_position[2]],
            [p_y_axis[0], p_y_axis[1], p_y_axis[2]],
            [0.2, 1.0, 0.2, 1.0]
        );

        debug_renderer.draw_line(
            [joint_position[0], joint_position[1], joint_position[2]],
            [p_z_axis[0], p_z_axis[1], p_z_axis[2]],
            [0.2, 0.2, 1.0, 1.0]
        );
    }
}

///
/// SQT - (Scale, Quaternion, Translation)
/// Transformation represented by separate scaling, translation, and rotation factors
/// Necessary for rotational interpolation
///
#[derive(Debug, Copy, Clone)]
pub struct SQT
{
    ///
    /// 3D Translation
    ///
    pub translation: Vector3<f32>,
    ///
    /// Uniform scale factor.
    ///
    pub scale: f32,
    ///
    /// Rotation
    ///
    pub rotation: Quaternion<f32>
}

#[derive(Debug)]
pub struct AnimationSample
{
    ///
    /// Local pose transforms for each joint in the targeted skeleton
    /// (relative to parent joint)
    ///
    pub local_poses: Vec<SQT>,
}