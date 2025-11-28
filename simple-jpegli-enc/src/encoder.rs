pub use crate::ffi::{ColorSpace, Subsampling};

use crate::ffi::{SJpegliConfig, encode_jpegli_internal};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum JpegError {
    #[error("Input buffer size mismatch. Expected {0}, got {1}")]
    BufferMismatch(usize, usize),
    #[error("Internal JPEGli error {code}: {message}")]
    JpegliError { code: i32, message: String },
}

pub struct JpegEncoder {
    quality: u8,
    subsampling: crate::ffi::Subsampling,
    progressive: bool,
    adaptive_quantize: bool,
    optimize_coding: bool,
    xyb_mode: bool,
    std_quant: bool,
}

impl Default for JpegEncoder {
    fn default() -> Self {
        Self {
            quality: 90,
            subsampling: crate::ffi::Subsampling::Auto,
            progressive: true,
            adaptive_quantize: true,
            optimize_coding: true,
            xyb_mode: false,
            std_quant: false,
        }
    }
}

impl JpegEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = quality.clamp(0, 100);
        self
    }

    pub fn subsampling(mut self, subsampling: crate::ffi::Subsampling) -> Self {
        self.subsampling = subsampling;
        self
    }

    pub fn progressive(mut self, progressive: bool) -> Self {
        self.progressive = progressive;
        self
    }

    pub fn adaptive_quantize(mut self, adaptive: bool) -> Self {
        self.adaptive_quantize = adaptive;
        self
    }

    pub fn optimize_coding(mut self, optimize: bool) -> Self {
        self.optimize_coding = optimize;
        self
    }

    pub fn xyb_mode(mut self, xyb_mode: bool) -> Self {
        self.xyb_mode = xyb_mode;
        self
    }

    pub fn std_quant(mut self, std_quant: bool) -> Self {
        self.std_quant = std_quant;
        self
    }

    pub fn encode(
        &self,
        data: &[u8],
        width: u16,
        height: u16,
        colorspace: ColorSpace,
        dpi: Option<(u16, u16)>,
    ) -> Result<Vec<u8>, JpegError> {
        // Basic validation
        let expected_len = (width as usize) * (height as usize) * colorspace.get_components();
        let pixels_len = data.len();

        if pixels_len != expected_len {
            return Err(JpegError::BufferMismatch(expected_len, pixels_len));
        }

        let mut config = SJpegliConfig::new(width.into(), height.into(), self.quality);
        config.set_color_space(colorspace);
        config.set_dpi(dpi.unwrap_or((72, 72)));
        config.set_subsampling(self.subsampling);
        config.set_adaptive_quantize(self.adaptive_quantize);
        config.set_optimize_coding(self.optimize_coding);
        config.set_progressive(self.progressive);
        config.set_xyb_mode(self.xyb_mode);
        config.set_std_quant(self.std_quant);

        match encode_jpegli_internal(data, &config) {
            Ok(encoded) => Ok(encoded),
            Err(err) => Err(JpegError::JpegliError {
                code: err.code,
                message: err.message,
            }),
        }
    }
}
