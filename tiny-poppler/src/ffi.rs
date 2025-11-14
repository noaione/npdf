use std::ffi::{CStr, CString, c_void};
use std::mem::MaybeUninit;
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
#[derive(Clone, Copy, Debug)]
#[expect(dead_code)]
enum ImageColorSpace {
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
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PdfCropMode {
    MediaBox = 0,
    #[default]
    CropBox = 1,
    BleedBox = 2,
    TrimBox = 3,
    ArtBox = 4,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Unknown = 0,
    Image = 1,
    Stencil = 2,
    Mask = 3,
    SoftMask = 4,
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
    page_number: u32,
    image_type: ImageType,
    colorspace: ImageColorSpace,
    color_space_handle: *const c_void,
}

/// Colorspace related
#[repr(C)]
struct ColorspaceIndexedInfo {
    hival: u32,
    base: *const c_void,
}

#[repr(C)]
struct ColorspaceSeparationInfo {
    name: *const c_char,
    alternate: *const c_void,
}

#[repr(C)]
struct ColorspaceDeviceNInfo {
    count: u32,
    names: *const *const c_char,
    alternate: *const c_void,
}

#[repr(C)]
struct ColorspaceLabXYZInfo {
    white_x: f64,
    white_y: f64,
    white_z: f64,
    black_x: f64,
    black_y: f64,
    black_z: f64,
    min_a: f64,
    min_b: f64,
    max_a: f64,
    max_b: f64,
}

#[repr(C)]
struct ColorspaceICCInfo {
    alternate: *const c_void,
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
        crop_mode: PdfCropMode,
        out_image: *mut SplashImage,
        error_out: *mut *mut c_char,
    ) -> i32;
    fn splash_renderer_free_image(image: *mut SplashImage);
    fn splash_renderer_free_cstr(message: *mut c_char);
    fn splash_renderer_collect_images(
        renderer: *mut SplashRenderer,
        out_images: *mut *mut SplashImageInfo,
        out_len: *mut usize,
        page_start: c_uint,
        page_end: c_uint,
        error_out: *mut *mut c_char,
    ) -> i32;
    fn splash_renderer_free_image_info(images: *mut SplashImageInfo);

    /// Colorspace related
    fn gfxcs_get_color_mode(ptr: *const c_void) -> ImageColorSpace;
    fn gfxcs_get_indexed_info(ptr: *const c_void, out: *mut ColorspaceIndexedInfo) -> bool;
    fn gfxcs_get_separation_info(ptr: *const c_void, out: *mut ColorspaceSeparationInfo) -> bool;
    fn gfxcs_get_devicen_info(ptr: *const c_void, out: *mut ColorspaceDeviceNInfo) -> bool;
    fn gfxcs_get_labxyz_info(ptr: *const c_void, out: *mut ColorspaceLabXYZInfo) -> bool;
    fn gfxcs_get_icc_info(ptr: *const c_void, out: *mut ColorspaceICCInfo) -> bool;

    fn gfxcs_free_string(s: *const c_char);
    fn gfxcs_free_string_array(arr: *const *const c_char, count: c_uint);
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

    pub fn collect_images(&mut self, range: Option<(u32, u32)>) -> Result<Vec<ImageInfo>, String> {
        let mut infos_ptr: *mut SplashImageInfo = ptr::null_mut();
        let mut len: usize = 0;
        let mut error = ptr::null_mut();
        let (start, end) = range.unwrap_or((0, 0));
        if start != 0 && end != 0 && end < start {
            return Err("invalid page range".into());
        }
        let status = unsafe {
            splash_renderer_collect_images(
                self.raw,
                &mut infos_ptr,
                &mut len,
                start,
                end,
                &mut error,
            )
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
        crop_mode: PdfCropMode,
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
                self.raw, page_index, dpi, color_mode, crop_mode, &mut image, &mut error,
            )
        };
        if status != 0 {
            return Err(take_error(error));
        }
        if image.data.is_null() || image.len == 0 {
            unsafe { splash_renderer_free_image(&mut image) };
            return Err("renderer returned empty bitmap".into());
        }
        let width = image.width;
        let height = image.height;
        let stride = image.stride;
        let components = image.components;
        let color_mode = image.color_mode;
        let bits_per_component = image.bits_per_component;
        let pixels = unsafe { slice::from_raw_parts(image.data, image.len) };
        let mut data = Vec::with_capacity(image.len);
        data.extend_from_slice(pixels);
        unsafe { splash_renderer_free_image(&mut image) };
        Ok(Image {
            data,
            width,
            height,
            stride,
            components,
            color_mode,
            bits_per_component,
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
#[derive(Clone)]
pub struct Image {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub components: u32,
    pub color_mode: ColorMode,
    pub bits_per_component: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct XYZColor {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct MinMaxColorRange {
    pub min: f64,
    pub max: f64,
}

/// Metadata about the colorspaces used by images.
#[derive(Debug, Clone)]
pub enum PdfImageColorSpace {
    Unknown,
    DeviceGray,
    DeviceRGB,
    DeviceCMYK,
    Lab {
        white: XYZColor,
        black: XYZColor,
        a: MinMaxColorRange,
        b: MinMaxColorRange,
    },
    ICC {
        alternate: Box<PdfImageColorSpace>,
    },
    Indexed {
        hival: u32,
        base: Box<PdfImageColorSpace>,
    },
    Pattern,
    Separation {
        name: String,
        alternate: Box<PdfImageColorSpace>,
    },
    DeviceN {
        count: u32,
        names: Vec<String>,
        alternate: Box<PdfImageColorSpace>,
    },
}

#[derive(Debug, Clone)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub components: u32,
    pub bits_per_component: u32,
    pub colorspace: PdfImageColorSpace,
    pub image_type: ImageType,
    pub page: u32,
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
            colorspace: convert_colorspace(value.color_space_handle),
            image_type: value.image_type,
            page: value.page_number,
            xref,
        }
    }
}

fn convert_colorspace(cs: *const c_void) -> PdfImageColorSpace {
    // if NULL pointer, return Unknown immediately
    if cs.is_null() {
        return PdfImageColorSpace::Unknown;
    }

    let kind = unsafe { gfxcs_get_color_mode(cs) };

    match kind {
        ImageColorSpace::DeviceGray => PdfImageColorSpace::DeviceGray,
        ImageColorSpace::DeviceRgb => PdfImageColorSpace::DeviceRGB,
        ImageColorSpace::DeviceCmyk => PdfImageColorSpace::DeviceCMYK,
        ImageColorSpace::Lab => {
            let mut info = MaybeUninit::<ColorspaceLabXYZInfo>::uninit();
            let ok = unsafe { gfxcs_get_labxyz_info(cs, info.as_mut_ptr()) };
            if !ok {
                return PdfImageColorSpace::Unknown;
            }
            let info = unsafe { info.assume_init() };

            PdfImageColorSpace::Lab {
                white: XYZColor {
                    x: info.white_x,
                    y: info.white_y,
                    z: info.white_z,
                },
                black: XYZColor {
                    x: info.black_x,
                    y: info.black_y,
                    z: info.black_z,
                },
                a: MinMaxColorRange {
                    min: info.min_a,
                    max: info.max_a,
                },
                b: MinMaxColorRange {
                    min: info.min_b,
                    max: info.max_b,
                },
            }
        }
        ImageColorSpace::Icc => {
            let mut info = MaybeUninit::<ColorspaceICCInfo>::uninit();
            let ok = unsafe { gfxcs_get_icc_info(cs, info.as_mut_ptr()) };
            if !ok {
                return PdfImageColorSpace::Unknown;
            }
            let info = unsafe { info.assume_init() };

            PdfImageColorSpace::ICC {
                alternate: Box::new(convert_colorspace(info.alternate)),
            }
        }
        ImageColorSpace::Pattern => PdfImageColorSpace::Pattern,

        ImageColorSpace::Indexed => {
            let mut info = MaybeUninit::<ColorspaceIndexedInfo>::uninit();
            let ok = unsafe { gfxcs_get_indexed_info(cs, info.as_mut_ptr()) };
            if !ok {
                return PdfImageColorSpace::Unknown;
            }
            let info = unsafe { info.assume_init() };

            PdfImageColorSpace::Indexed {
                hival: info.hival,
                base: Box::new(convert_colorspace(info.base)),
            }
        }

        ImageColorSpace::Separation => {
            let mut info = MaybeUninit::<ColorspaceSeparationInfo>::uninit();
            let ok = unsafe { gfxcs_get_separation_info(cs, info.as_mut_ptr()) };
            if !ok {
                return PdfImageColorSpace::Unknown;
            }
            let info = unsafe { info.assume_init() };

            let name = unsafe { CStr::from_ptr(info.name).to_string_lossy().into_owned() };
            unsafe { gfxcs_free_string(info.name) };

            PdfImageColorSpace::Separation {
                name,
                alternate: Box::new(convert_colorspace(info.alternate)),
            }
        }

        ImageColorSpace::DeviceN => {
            let mut info = MaybeUninit::<ColorspaceDeviceNInfo>::uninit();
            let ok = unsafe { gfxcs_get_devicen_info(cs, info.as_mut_ptr()) };
            if !ok {
                return PdfImageColorSpace::Unknown;
            }
            let info = unsafe { info.assume_init() };

            let mut names = Vec::new();
            for i in 0..info.count {
                let ptr = unsafe { *info.names.add(i as usize) };
                unsafe { names.push(CStr::from_ptr(ptr).to_string_lossy().into_owned()) };
            }

            unsafe { gfxcs_free_string_array(info.names, info.count) };

            PdfImageColorSpace::DeviceN {
                count: info.count,
                names,
                alternate: Box::new(convert_colorspace(info.alternate)),
            }
        }

        _ => PdfImageColorSpace::Unknown,
    }
}
