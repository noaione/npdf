pub use crate::ffi::ColorSpace;

use crate::ffi::encode_jpegli_internal;
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
}

impl Default for JpegEncoder {
    fn default() -> Self {
        Self { quality: 90 }
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

        let dpi = dpi.unwrap_or((72, 72));

        match encode_jpegli_internal(
            data,
            width.into(),
            height.into(),
            self.quality,
            colorspace,
            dpi.0,
            dpi.1,
        ) {
            Ok(encoded) => Ok(encoded),
            Err(err) => Err(JpegError::JpegliError {
                code: err.code,
                message: err.message,
            }),
        }
    }
}
