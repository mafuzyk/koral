use koral::render::surface_format;
use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba16Float};

#[test]
fn a_float_surface_is_not_chosen_because_its_colours_are_linear() {
    assert_eq!(surface_format(&[Rgba16Float, Bgra8UnormSrgb, Bgra8Unorm]), Bgra8Unorm);
}

#[test]
fn a_surface_without_bgra_uses_rgba() {
    assert_eq!(surface_format(&[Rgba16Float, Rgba8Unorm]), Rgba8Unorm);
}

#[test]
fn a_picture_keeps_its_shape_inside_its_cells() {
    assert_eq!(koral::frame::fit_picture((100, 50), (100.0, 60.0)), (100.0, 50.0));
    assert_eq!(koral::frame::fit_picture((16, 16), (8.0, 24.0)), (8.0, 8.0));
}
