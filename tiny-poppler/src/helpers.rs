use png::PixelDimensions;

pub(crate) fn build_pixel_dims_png(x_dpi: f64, y_dpi: f64) -> Option<PixelDimensions> {
    let xppu = dpi_to_pixels_per_meter(x_dpi)?;
    let yppu = dpi_to_pixels_per_meter(y_dpi)?;
    Some(PixelDimensions {
        xppu,
        yppu,
        unit: png::Unit::Meter,
    })
}

pub(crate) fn dpi_to_pixels_per_meter(value: f64) -> Option<u32> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let ppm = (value / 0.0254).round();
    if ppm < 1.0 || ppm > u32::MAX as f64 {
        return None;
    }
    Some(ppm as u32)
}
