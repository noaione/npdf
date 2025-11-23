use crate::ffi;
use std::ffi::CStr;
use std::hint::unreachable_unchecked;
use std::mem;
use std::ptr;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    RGB,
    Grayscale,
    CMYK,
}

#[derive(Error, Debug)]
pub enum JpegError {
    #[error("Failed to create compression structure")]
    CreateCompressFailed,
    #[error("Failed to start compression")]
    StartCompressFailed,
    #[error("Failed to write scanlines")]
    WriteScanlinesFailed,
    #[error("Internal JPEG error {code}: {message}")]
    InternalError {
        code: i32,
        message: String,
        warnings: Vec<String>,
    },
}

const ERROR_MSG_LEN: usize = ffi::JMSG_LENGTH_MAX as usize;

#[inline(always)]
unsafe fn jpegli_setjmp(env: *mut ffi::rust_jpegli_jmp_buf) -> libc::c_int {
    unsafe { ffi::rust_jpegli_setjmp(env) }
}

#[inline(always)]
unsafe fn jpegli_longjmp(env: *mut ffi::rust_jpegli_jmp_buf, value: libc::c_int) -> ! {
    unsafe {
        ffi::rust_jpegli_longjmp(env, value);
        unreachable_unchecked();
    }
}

#[repr(C)]
struct ErrorHandler {
    base: ffi::jpeg_error_mgr,
    jump_buffer: ffi::rust_jpegli_jmp_buf,
    message: [libc::c_char; ERROR_MSG_LEN],
    compressor_created: bool,
    out_buffer: *mut u8,
    warnings: Vec<String>,
    last_error_code: libc::c_int,
    fallback_emit_message: Option<unsafe extern "C" fn(ffi::j_common_ptr, libc::c_int)>,
}

impl Default for ErrorHandler {
    fn default() -> Self {
        Self {
            base: unsafe { mem::zeroed() },
            jump_buffer: unsafe { mem::zeroed() },
            message: [0; ERROR_MSG_LEN],
            compressor_created: false,
            out_buffer: ptr::null_mut(),
            warnings: Vec::new(),
            last_error_code: 0,
            fallback_emit_message: None,
        }
    }
}

impl ErrorHandler {
    fn as_mut_ptr(&mut self) -> *mut ffi::jpeg_error_mgr {
        &mut self.base
    }

    fn jump_buffer_mut(&mut self) -> *mut ffi::rust_jpegli_jmp_buf {
        &mut self.jump_buffer as *mut _
    }

    fn mark_compressor_created(&mut self) {
        self.compressor_created = true;
    }

    fn compressor_ready(&self) -> bool {
        self.compressor_created
    }

    fn store_output_buffer(&mut self, buffer: *mut u8) {
        self.out_buffer = buffer;
    }

    fn take_output_buffer(&mut self) -> *mut u8 {
        let buffer = self.out_buffer;
        self.out_buffer = ptr::null_mut();
        buffer
    }

    fn message(&self) -> String {
        unsafe {
            CStr::from_ptr(self.message.as_ptr())
                .to_string_lossy()
                .into_owned()
        }
    }

    fn take_warnings(&mut self) -> Vec<String> {
        mem::take(&mut self.warnings)
    }

    fn record_error_code(&mut self) {
        self.last_error_code = self.base.msg_code;
    }

    fn install_emit_hook(&mut self) {
        self.fallback_emit_message = self.base.emit_message;
        self.base.emit_message = Some(track_emit_message);
    }

    fn take_internal_error(&mut self) -> JpegError {
        JpegError::InternalError {
            code: self.last_error_code,
            message: self.message(),
            warnings: self.take_warnings(),
        }
    }
}

unsafe extern "C" fn track_emit_message(cinfo: ffi::j_common_ptr, msg_level: libc::c_int) {
    let err_ptr = unsafe { (*cinfo).err as *mut ErrorHandler };
    let err = unsafe { &mut *err_ptr };

    if msg_level > 0 {
        if let Some(format_message) = err.base.format_message {
            let mut buffer = [0 as libc::c_char; ERROR_MSG_LEN];
            unsafe {
                format_message(cinfo, buffer.as_mut_ptr());
                let warning = CStr::from_ptr(buffer.as_ptr())
                    .to_string_lossy()
                    .into_owned();
                err.warnings.push(warning);
            }
        }
    }

    if let Some(fallback) = err.fallback_emit_message {
        unsafe {
            fallback(cinfo, msg_level);
        }
    }
}

unsafe extern "C" fn error_exit(cinfo: ffi::j_common_ptr) {
    let err_ptr = unsafe { (*cinfo).err as *mut ErrorHandler };
    let err = unsafe { &mut *err_ptr };

    err.record_error_code();

    if let Some(format_message) = err.base.format_message {
        unsafe {
            format_message(cinfo, err.message.as_mut_ptr());
        }
    } else {
        err.message = [0; ERROR_MSG_LEN];
    }

    unsafe {
        jpegli_longjmp(err.jump_buffer_mut(), 1);
    }
}

pub struct JpegEncoder {
    quality: u8,
    progressive: bool,
    optimize_coding: bool,
}

impl Default for JpegEncoder {
    fn default() -> Self {
        Self {
            quality: 90,
            progressive: true,
            optimize_coding: true,
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

    pub fn progressive(mut self, progressive: bool) -> Self {
        self.progressive = progressive;
        self
    }

    pub fn optimize_coding(mut self, optimize: bool) -> Self {
        self.optimize_coding = optimize;
        self
    }

    pub fn encode(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        colorspace: ColorSpace,
    ) -> Result<Vec<u8>, JpegError> {
        unsafe {
            let mut cinfo: ffi::jpeg_compress_struct = mem::zeroed();
            let mut err = ErrorHandler::default();

            // Set up error handling
            cinfo.err = ffi::jpeg_std_error(err.as_mut_ptr());
            err.install_emit_hook();
            err.base.error_exit = Some(error_exit);

            if jpegli_setjmp(err.jump_buffer_mut()) != 0 {
                if err.compressor_ready() {
                    ffi::jpeg_destroy_compress(&mut cinfo);
                }
                let buffer = err.take_output_buffer();
                if !buffer.is_null() {
                    libc::free(buffer as *mut libc::c_void);
                }
                return Err(err.take_internal_error());
            }

            // Initialize compression object
            ffi::jpeg_CreateCompress(
                &mut cinfo,
                ffi::JPEG_LIB_VERSION as i32,
                mem::size_of::<ffi::jpeg_compress_struct>(),
            );
            err.mark_compressor_created();

            // Use a memory destination
            let mut out_buffer: *mut u8 = ptr::null_mut();
            let mut out_size: libc::c_ulong = 0;
            ffi::jpeg_mem_dest(&mut cinfo, &mut out_buffer, &mut out_size);
            err.store_output_buffer(out_buffer);

            // Set image parameters
            cinfo.image_width = width;
            cinfo.image_height = height;
            cinfo.input_components = match colorspace {
                ColorSpace::RGB => 3,
                ColorSpace::Grayscale => 1,
                ColorSpace::CMYK => 4,
            } as i32;
            cinfo.in_color_space = match colorspace {
                ColorSpace::RGB => ffi::J_COLOR_SPACE_JCS_RGB,
                ColorSpace::Grayscale => ffi::J_COLOR_SPACE_JCS_GRAYSCALE,
                ColorSpace::CMYK => ffi::J_COLOR_SPACE_JCS_CMYK,
            };

            ffi::jpeg_set_defaults(&mut cinfo);

            // Set compression parameters
            ffi::jpeg_set_quality(&mut cinfo, self.quality as i32, 1); // 1 = force_baseline (TRUE)
            if self.progressive {
                ffi::jpeg_simple_progression(&mut cinfo);
            }
            cinfo.optimize_coding = if self.optimize_coding { 1 } else { 0 };

            // Start compression
            ffi::jpeg_start_compress(&mut cinfo, 1); // 1 = TRUE

            // Write scanlines
            let row_stride = (width * cinfo.input_components as u32) as usize;
            while cinfo.next_scanline < cinfo.image_height {
                let row_ptr = data.as_ptr().add(cinfo.next_scanline as usize * row_stride);
                // jpeg_write_scanlines expects JSAMPARRAY which is JSAMPROW * which is unsigned char **
                // But we have const u8 *. We need to cast it to *mut u8 because the API is not const-correct in C
                let mut row_pointer = [row_ptr as *mut u8];
                let written = ffi::jpeg_write_scanlines(&mut cinfo, row_pointer.as_mut_ptr(), 1);
                if written != 1 {
                    ffi::jpeg_finish_compress(&mut cinfo);
                    ffi::jpeg_destroy_compress(&mut cinfo);
                    let buffer = err.take_output_buffer();
                    if !buffer.is_null() {
                        libc::free(buffer as *mut libc::c_void);
                    }
                    return Err(JpegError::WriteScanlinesFailed);
                }
            }

            // Finish compression
            ffi::jpeg_finish_compress(&mut cinfo);
            ffi::jpeg_destroy_compress(&mut cinfo);

            // Copy the result to a Vec
            let result = std::slice::from_raw_parts(out_buffer, out_size as usize).to_vec();

            // Free the buffer allocated by jpeg_mem_dest (it uses malloc)
            libc::free(out_buffer as *mut libc::c_void);
            err.store_output_buffer(ptr::null_mut());

            Ok(result)
        }
    }
}
