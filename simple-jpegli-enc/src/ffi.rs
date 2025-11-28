use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uchar, c_uint, c_ulong};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Grayscale = 0,
    Rgb = 1,
    YCbCr = 2,
    Cmyk = 3,
    Ycck = 4,
    ExtRgb = 5,
    ExtRgbx = 6,
    ExtBgr = 7,
    ExtBgrx = 8,
    ExtXbgr = 9,
    ExtXrgb = 10,
    ExtRgba = 11,
    ExtBgra = 12,
    ExtAbgr = 13,
    ExtArgb = 14,
    Rgb565 = 15,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsampling {
    None = 0,
    Auto = 1,
    Sub420 = 2,
    Sub422 = 3,
    Sub440 = 4,
    Sub444 = 5,
}

impl ColorSpace {
    pub fn get_components(&self) -> usize {
        match self {
            ColorSpace::Grayscale => 1,
            ColorSpace::Rgb | ColorSpace::YCbCr | ColorSpace::ExtRgb | ColorSpace::ExtBgr => 3,
            ColorSpace::Cmyk | ColorSpace::Ycck => 4,
            ColorSpace::ExtRgbx
            | ColorSpace::ExtBgrx
            | ColorSpace::ExtXbgr
            | ColorSpace::ExtXrgb
            | ColorSpace::ExtRgba
            | ColorSpace::ExtBgra
            | ColorSpace::ExtAbgr
            | ColorSpace::ExtArgb => 4,
            ColorSpace::Rgb565 => 2,
        }
    }
}

// 1. Define the C Layout
// This must match jpegli_wrapper.h EXACTLY
const ERR_MSG_LEN: usize = 256;

#[repr(C)]
struct SJpegliResult {
    data: *mut c_uchar,
    size: c_ulong,
    success: c_int,
    error_code: c_int,
    error_message: [c_char; ERR_MSG_LEN],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct SJpegliConfig {
    width: c_int,
    height: c_int,
    quality: c_int,
    x_dpi: c_uint,
    y_dpi: c_uint,
    color_space: ColorSpace,
    subsampling: Subsampling,
    progressive: bool,
    adaptive_quantize: bool,
    xyb_mode: bool,
    std_quant: bool,
    optimize_coding: bool,
}

impl SJpegliConfig {
    pub fn new(width: c_int, height: c_int, quality: u8) -> Self {
        SJpegliConfig {
            width,
            height,
            quality: quality.into(),
            x_dpi: 72,
            y_dpi: 72,
            color_space: ColorSpace::Rgb,
            subsampling: Subsampling::Auto,
            progressive: true,
            adaptive_quantize: true,
            xyb_mode: false,
            std_quant: false,
            optimize_coding: true,
        }
    }

    pub fn set_color_space(&mut self, color_space: ColorSpace) {
        self.color_space = color_space;
    }

    pub fn set_subsampling(&mut self, subsampling: Subsampling) {
        self.subsampling = subsampling;
    }

    pub fn set_dpi(&mut self, dpi: (u16, u16)) {
        self.x_dpi = dpi.0.into();
        self.y_dpi = dpi.1.into();
    }

    pub fn set_progressive(&mut self, progressive: bool) {
        self.progressive = progressive;
    }

    pub fn set_adaptive_quantize(&mut self, adaptive: bool) {
        self.adaptive_quantize = adaptive;
    }

    pub fn set_xyb_mode(&mut self, xyb_mode: bool) {
        self.xyb_mode = xyb_mode;
    }

    pub fn set_std_quant(&mut self, std_quant: bool) {
        self.std_quant = std_quant;
    }

    pub fn set_optimize_coding(&mut self, optimize: bool) {
        self.optimize_coding = optimize;
    }
}

impl SJpegliResult {
    fn is_success(&self) -> bool {
        self.success != 0
    }

    fn get_error_message(&self) -> String {
        unsafe {
            let c_str = CStr::from_ptr(self.error_message.as_ptr());
            c_str.to_string_lossy().into_owned()
        }
    }
}

unsafe extern "C" {
    fn sjpegli_encode_pixels(pixels: *const c_uchar, config: *const SJpegliConfig)
    -> SJpegliResult;

    fn sjpegli_free_result(result: SJpegliResult);
}

// 2. High-Level Rust Error
#[derive(Debug, Clone)]
pub(crate) struct SJpegliError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for SJpegliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Jpegli Error (Code {}): {}", self.code, self.message)
    }
}
impl std::error::Error for SJpegliError {}

pub(crate) fn encode_jpegli_internal(
    pixels: &[u8],
    config: &SJpegliConfig,
) -> Result<Vec<u8>, SJpegliError> {
    unsafe {
        println!("Calling sjpegli_encode_pixels with config: {:?}", config);
        let result = sjpegli_encode_pixels(pixels.as_ptr(), config);
        println!(
            "Received SJpegliResult: size={}, success={}, error_code={}",
            result.size, result.success, result.error_code
        );

        if !result.is_success() {
            // Convert C string to Rust string
            let error_message = result.get_error_message();
            let error_code = result.error_code;

            sjpegli_free_result(result);

            return Err(SJpegliError {
                code: error_code,
                message: error_message,
            });
        }

        let output_slice = std::slice::from_raw_parts(result.data, result.size as usize);
        let owned_vec = output_slice.to_vec();
        sjpegli_free_result(result); // No memory leak
        Ok(owned_vec)
    }
}
