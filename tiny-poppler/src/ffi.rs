use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_uint};
use std::path::Path;
use std::ptr;
use std::slice;

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[repr(C)]
struct SplashRenderer {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Mono1 = 0,
    Mono8 = 1,
    Rgb8 = 2,
    Bgr8 = 3,
    Xbgr8 = 4,
    Cmyk8 = 5,
    DeviceN8 = 6,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageColorSpace {
    Unknown = 0,
    DeviceGray = 1,
    DeviceRgb = 2,
    DeviceCmyk = 3,
    Lab = 4,
    Icc = 5,
    Indexed = 6,
    Pattern = 7,
    Separation = 8,
    DeviceN = 9,
    Other = 10,
}

#[repr(C)]
struct SplashImage {
    data: *mut u8,
    len: usize,
    width: u32,
    height: u32,
    stride: u32,
    components: u32,
    color_mode: ColorMode,
    bits_per_component: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct SplashImageInfo {
    width: u32,
    height: u32,
    components: u32,
    bits_per_component: u32,
    xref_object: i32,
    xref_generation: i32,
    colorspace: ImageColorSpace,
}

unsafe extern "C" {
    fn splash_renderer_create(
        path: *const c_char,
        out_renderer: *mut *mut SplashRenderer,
        error_out: *mut *mut c_char,
    ) -> i32;
    fn splash_renderer_destroy(renderer: *mut SplashRenderer);
    fn splash_renderer_page_count(
        renderer: *mut SplashRenderer,
        out_count: *mut c_uint,
        error_out: *mut *mut c_char,
    ) -> i32;
    fn splash_renderer_render_page(
        renderer: *mut SplashRenderer,
        page_index: c_uint,
        dpi: c_double,
        color_mode: ColorMode,
        out_image: *mut SplashImage,
        error_out: *mut *mut c_char,
    ) -> i32;
    fn splash_renderer_free_image(image: *mut SplashImage);
    fn splash_renderer_free_cstr(message: *mut c_char);
    fn splash_renderer_collect_images(
        renderer: *mut SplashRenderer,
        out_images: *mut *mut SplashImageInfo,
        out_len: *mut usize,
        error_out: *mut *mut c_char,
    ) -> i32;
    fn splash_renderer_free_image_info(images: *mut SplashImageInfo);
}

fn take_error(message: *mut c_char) -> String {
    if message.is_null() {
        return "unknown poppler error".to_string();
    }
    unsafe {
        let c_str = CStr::from_ptr(message);
        let string = c_str.to_string_lossy().into_owned();
        splash_renderer_free_cstr(message);
        string
    }
}

fn path_to_cstring(path: &Path) -> Result<CString, String> {
    #[cfg(unix)]
    {
        let bytes = path.as_os_str().as_bytes();
        CString::new(bytes).map_err(|_| "path contains an internal NUL byte".to_string())
    }
    #[cfg(not(unix))]
    {
        path.to_str()
            .ok_or_else(|| "path is not valid UTF-8".to_string())
            .and_then(|s| {
                CString::new(s.as_bytes())
                    .map_err(|_| "path contains an internal NUL byte".to_string())
            })
    }
}

/// Safe wrapper over the Splash renderer.
pub struct Renderer {
    raw: *mut SplashRenderer,
}

impl Renderer {
    pub fn open(path: &Path) -> Result<Self, String> {
        let c_path = path_to_cstring(path)?;
        let mut raw = ptr::null_mut();
        let mut error = ptr::null_mut();
        let status = unsafe { splash_renderer_create(c_path.as_ptr(), &mut raw, &mut error) };
        if status != 0 {
            return Err(take_error(error));
        }
        if raw.is_null() {
            return Err("poppler returned a null renderer".into());
        }
        Ok(Self { raw })
    }

    pub fn page_count(&self) -> Result<u32, String> {
        let mut count: c_uint = 0;
        let mut error = ptr::null_mut();
        let status = unsafe { splash_renderer_page_count(self.raw, &mut count, &mut error) };
        if status != 0 {
            return Err(take_error(error));
        }
        Ok(count)
    }

    pub fn collect_images(&mut self) -> Result<Vec<ImageInfo>, String> {
        let mut infos_ptr: *mut SplashImageInfo = ptr::null_mut();
        let mut len: usize = 0;
        let mut error = ptr::null_mut();
        let status = unsafe {
            splash_renderer_collect_images(self.raw, &mut infos_ptr, &mut len, &mut error)
        };
        if status != 0 {
            return Err(take_error(error));
        }
        if infos_ptr.is_null() || len == 0 {
            if !infos_ptr.is_null() {
                unsafe { splash_renderer_free_image_info(infos_ptr) };
            }
            return Ok(Vec::new());
        }
        let slice = unsafe { slice::from_raw_parts(infos_ptr, len) };
        let mut images = Vec::with_capacity(slice.len());
        for info in slice {
            images.push(ImageInfo::from(*info));
        }
        unsafe { splash_renderer_free_image_info(infos_ptr) };
        Ok(images)
    }

    pub fn render_page(
        &mut self,
        page_index: u32,
        dpi: f64,
        color_mode: ColorMode,
    ) -> Result<Image, String> {
        let mut image = SplashImage {
            data: ptr::null_mut(),
            len: 0,
            width: 0,
            height: 0,
            stride: 0,
            components: 0,
            color_mode: ColorMode::Rgb8,
            bits_per_component: 0,
        };
        let mut error = ptr::null_mut();
        let status = unsafe {
            splash_renderer_render_page(
                self.raw, page_index, dpi, color_mode, &mut image, &mut error,
            )
        };
        if status != 0 {
            return Err(take_error(error));
        }
        if image.data.is_null() || image.len == 0 {
            unsafe { splash_renderer_free_image(&mut image) };
            return Err("renderer returned empty bitmap".into());
        }
        let pixels = unsafe { slice::from_raw_parts(image.data, image.len) };
        let mut data = Vec::with_capacity(image.len);
        data.extend_from_slice(pixels);
        unsafe { splash_renderer_free_image(&mut image) };
        Ok(Image {
            data,
            width: image.width,
            height: image.height,
            stride: image.stride,
            components: image.components,
            color_mode: image.color_mode,
            bits_per_component: image.bits_per_component,
        })
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { splash_renderer_destroy(self.raw) };
        }
    }
}

/// Raw bitmap returned by Splash.
#[derive(Debug, Clone)]
pub struct Image {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub components: u32,
    pub color_mode: ColorMode,
    pub bits_per_component: u32,
}

#[derive(Debug, Clone)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub components: u32,
    pub bits_per_component: u32,
    pub colorspace: ImageColorSpace,
    pub xref: Option<(i32, i32)>,
}

impl From<SplashImageInfo> for ImageInfo {
    fn from(value: SplashImageInfo) -> Self {
        let xref = if value.xref_object >= 0 {
            Some((value.xref_object, value.xref_generation))
        } else {
            None
        };
        Self {
            width: value.width,
            height: value.height,
            components: value.components,
            bits_per_component: value.bits_per_component,
            colorspace: value.colorspace,
            xref,
        }
    }
}
