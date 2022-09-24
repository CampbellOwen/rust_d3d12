use d3d12_utils::{MeshHandle, TextureHandle};
use glam::Vec3;

#[derive(Debug)]
pub struct Object {
    pub position: Vec3,
    pub texture: TextureHandle,
    pub mesh: MeshHandle,
}
