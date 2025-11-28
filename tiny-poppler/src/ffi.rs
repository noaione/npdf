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
    // BleedBox = 2,
    // TrimBox = 3,
    // ArtBox = 4,
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
    dpi_x: f64,
    dpi_y: f64,
    image_type: ImageType,
    colorspace: ImageColorSpace,
    color_space_handle: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct SplashPageInfo {
    page_number: u32,
    image_count: u32,
    object_count: u64,
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

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageExportMatchMode {
    ByRef = 0,
    ByOccurrence = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageExportType {
    Image = 0,
    Stencil = 1,
    Mask = 2,
    SoftMask = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageExportFormat {
    Unknown = 0,
    Rgb = 1,
    Rgb48 = 2,
    Gray = 3,
    Monochrome = 4,
    Cmyk = 5,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageExportExtension {
    Jpeg = 0,
    Jp2 = 1,
    Jbig2 = 2,
    Ccitt = 3,
    Png = 4,
    Tiff = 5,
    Pnm = 6,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct ImageExportParams {
    page_index: u32,
    match_mode: ImageExportMatchMode,
    target_type: ImageExportType,
    xref_object: i32,
    xref_generation: i32,
    occurrence_index: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct ImageCcittParams {
    encoding: i32,
    columns: i32,
    rows: i32,
    damaged_rows_before_error: i32,
    end_of_line: u8,
    byte_align: u8,
    end_of_block: u8,
    black_is_one: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct ImageExportImage {
    data: *mut u8,
    len: usize,
    width: u32,
    height: u32,
    stride: u32,
    components: u32,
    bits_per_component: u32,
    width_dpi: f64,
    height_dpi: f64,
    format: ImageExportFormat,
    image_type: ImageExportType,
    extension: ImageExportExtension,
    jbig2_globals: *mut u8,
    jbig2_globals_len: usize,
    has_ccitt_params: u8,
    ccitt: ImageCcittParams,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VersionInfo {
    major: u32,
    minor: u32,
    patch: u32,
}

unsafe extern "C" {
    fn splash_renderer_create(
        path: *const c_char,
        owner_password: *const c_char,
        user_password: *const c_char,
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
        out_pages: *mut *mut SplashPageInfo,
        out_page_len: *mut usize,
        page_start: c_uint,
        page_end: c_uint,
        error_out: *mut *mut c_char,
    ) -> i32;
    fn splash_renderer_free_image_info(images: *mut SplashImageInfo);
    fn splash_renderer_free_page_info(pages: *mut SplashPageInfo);
    fn splash_get_version(out_version: *mut VersionInfo);

    /// Colorspace related
    fn gfxcs_get_color_mode(ptr: *const c_void) -> ImageColorSpace;
    fn gfxcs_get_indexed_info(ptr: *const c_void, out: *mut ColorspaceIndexedInfo) -> bool;
    fn gfxcs_get_separation_info(ptr: *const c_void, out: *mut ColorspaceSeparationInfo) -> bool;
    fn gfxcs_get_devicen_info(ptr: *const c_void, out: *mut ColorspaceDeviceNInfo) -> bool;
    fn gfxcs_get_labxyz_info(ptr: *const c_void, out: *mut ColorspaceLabXYZInfo) -> bool;
    fn gfxcs_get_icc_info(ptr: *const c_void, out: *mut ColorspaceICCInfo) -> bool;

    fn gfxcs_free_string(s: *const c_char);
    fn gfxcs_free_string_array(arr: *const *const c_char, count: c_uint);

    fn image_exporter_extract(
        renderer: *mut SplashRenderer,
        params: *const ImageExportParams,
        out_image: *mut ImageExportImage,
        error_out: *mut *mut c_char,
    ) -> i32;
    fn image_exporter_free(image: *mut ImageExportImage);
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

fn optional_cstring(value: Option<&str>) -> Result<Option<CString>, String> {
    value
        .map(|password| {
            CString::new(password).map_err(|_| "password contains an internal NUL byte".to_string())
        })
        .transpose()
}

/// Safe wrapper over the Splash renderer.
pub struct Renderer {
    raw: *mut SplashRenderer,
}

impl Renderer {
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_with_passwords(path, None, None)
    }

    pub fn open_with_passwords(
        path: &Path,
        owner_password: Option<&str>,
        user_password: Option<&str>,
    ) -> Result<Self, String> {
        let c_path = path_to_cstring(path)?;
        let owner_c = optional_cstring(owner_password)?;
        let user_c = optional_cstring(user_password)?;
        let mut raw: *mut SplashRenderer = ptr::null_mut();
        let mut error = ptr::null_mut();
        let status = unsafe {
            splash_renderer_create(
                c_path.as_ptr(),
                owner_c
                    .as_ref()
                    .map_or(ptr::null(), |password| password.as_ptr()),
                user_c
                    .as_ref()
                    .map_or(ptr::null(), |password| password.as_ptr()),
                &mut raw,
                &mut error,
            )
        };
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

    pub fn collect_images(&mut self, range: Option<(u32, u32)>) -> Result<ImageCollection, String> {
        let mut infos_ptr: *mut SplashImageInfo = ptr::null_mut();
        let mut pages_ptr: *mut SplashPageInfo = ptr::null_mut();
        let mut image_len: usize = 0;
        let mut page_len: usize = 0;
        let mut error = ptr::null_mut();
        let (start, end) = range.unwrap_or((0, 0));
        if start != 0 && end != 0 && end < start {
            return Err("invalid page range".into());
        }
        let status = unsafe {
            splash_renderer_collect_images(
                self.raw,
                &mut infos_ptr,
                &mut image_len,
                &mut pages_ptr,
                &mut page_len,
                start,
                end,
                &mut error,
            )
        };
        if status != 0 {
            if !infos_ptr.is_null() {
                unsafe { splash_renderer_free_image_info(infos_ptr) };
            }
            if !pages_ptr.is_null() {
                unsafe { splash_renderer_free_page_info(pages_ptr) };
            }
            return Err(take_error(error));
        }

        let images = if !infos_ptr.is_null() && image_len > 0 {
            let slice = unsafe { slice::from_raw_parts(infos_ptr, image_len) };
            slice.iter().map(|info| ImageInfo::from(*info)).collect()
        } else {
            Vec::new()
        };
        if !infos_ptr.is_null() {
            unsafe { splash_renderer_free_image_info(infos_ptr) };
        }

        let pages = if !pages_ptr.is_null() && page_len > 0 {
            let slice = unsafe { slice::from_raw_parts(pages_ptr, page_len) };
            slice.iter().map(|info| PageInfo::from(*info)).collect()
        } else {
            Vec::new()
        };
        if !pages_ptr.is_null() {
            unsafe { splash_renderer_free_page_info(pages_ptr) };
        }

        Ok(ImageCollection { images, pages })
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

    pub fn export_image(&mut self, request: ImageExportRequest) -> Result<ExportedImage, String> {
        let (match_mode, xref_object, xref_generation, occurrence_index) = match request.selector {
            ImageExportSelector::Reference { object, generation } => {
                (ImageExportMatchMode::ByRef, object, generation, 0)
            }
            ImageExportSelector::NthOfType { occurrence } => {
                (ImageExportMatchMode::ByOccurrence, 0, 0, occurrence)
            }
        };

        let params = ImageExportParams {
            page_index: request.page_index,
            match_mode,
            target_type: request.target_type,
            xref_object,
            xref_generation,
            occurrence_index,
        };

        let mut raw = ImageExportImage {
            data: ptr::null_mut(),
            len: 0,
            width: 0,
            height: 0,
            stride: 0,
            components: 0,
            bits_per_component: 0,
            width_dpi: 0.0,
            height_dpi: 0.0,
            format: ImageExportFormat::Unknown,
            image_type: request.target_type,
            extension: ImageExportExtension::Png,
            jbig2_globals: ptr::null_mut(),
            jbig2_globals_len: 0,
            has_ccitt_params: 0,
            ccitt: ImageCcittParams {
                encoding: 0,
                columns: 0,
                rows: 0,
                damaged_rows_before_error: 0,
                end_of_line: 0,
                byte_align: 0,
                end_of_block: 0,
                black_is_one: 0,
            },
        };
        let mut error = ptr::null_mut();
        let status = unsafe { image_exporter_extract(self.raw, &params, &mut raw, &mut error) };
        if status != 0 {
            unsafe { image_exporter_free(&mut raw) };
            return Err(take_error(error));
        }
        if raw.len > 0 && raw.data.is_null() {
            unsafe { image_exporter_free(&mut raw) };
            return Err("image exporter returned an empty buffer".into());
        }

        let width = raw.width;
        let height = raw.height;
        let stride = raw.stride;
        let components = raw.components;
        let bits_per_component = raw.bits_per_component;
        let width_dpi = raw.width_dpi;
        let height_dpi = raw.height_dpi;
        let format = raw.format;
        let image_type = raw.image_type;
        let extension = raw.extension;

        let data = if raw.len == 0 {
            Vec::new()
        } else {
            let bytes = unsafe { slice::from_raw_parts(raw.data, raw.len) };
            let mut owned = Vec::with_capacity(bytes.len());
            owned.extend_from_slice(bytes);
            owned
        };

        let jbig2_globals = if raw.jbig2_globals_len == 0 || raw.jbig2_globals.is_null() {
            None
        } else {
            let bytes = unsafe { slice::from_raw_parts(raw.jbig2_globals, raw.jbig2_globals_len) };
            Some(bytes.to_vec())
        };

        let ccitt_params = if raw.has_ccitt_params == 0 {
            None
        } else {
            Some(CcittParams::from(raw.ccitt))
        };

        unsafe { image_exporter_free(&mut raw) };

        Ok(ExportedImage {
            data,
            width,
            height,
            stride,
            components,
            bits_per_component,
            width_dpi,
            height_dpi,
            format,
            image_type,
            extension,
            jbig2_globals,
            ccitt_params,
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

#[derive(Clone)]
pub struct ExportedImage {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub components: u32,
    pub bits_per_component: u32,
    pub width_dpi: f64,
    pub height_dpi: f64,
    pub format: ImageExportFormat,
    pub image_type: ImageExportType,
    pub extension: ImageExportExtension,
    pub jbig2_globals: Option<Vec<u8>>,
    pub ccitt_params: Option<CcittParams>,
}

impl std::fmt::Debug for ExportedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportedImage")
            .field("data[len]", &self.data.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stride", &self.stride)
            .field("components", &self.components)
            .field("bits_per_component", &self.bits_per_component)
            .field("width_dpi", &self.width_dpi)
            .field("height_dpi", &self.height_dpi)
            .field("format", &self.format)
            .field("image_type", &self.image_type)
            .field("extension", &self.extension)
            .field(
                "jbig2_globals[len]",
                &self.jbig2_globals.as_ref().map(|v| v.len()),
            )
            .field("ccitt_params", &self.ccitt_params)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcittParams {
    pub encoding: i32,
    pub columns: i32,
    pub rows: i32,
    pub damaged_rows_before_error: i32,
    pub end_of_line: bool,
    pub byte_align: bool,
    pub end_of_block: bool,
    pub black_is_one: bool,
}

impl From<ImageCcittParams> for CcittParams {
    fn from(value: ImageCcittParams) -> Self {
        Self {
            encoding: value.encoding,
            columns: value.columns,
            rows: value.rows,
            damaged_rows_before_error: value.damaged_rows_before_error,
            end_of_line: value.end_of_line != 0,
            byte_align: value.byte_align != 0,
            end_of_block: value.end_of_block != 0,
            black_is_one: value.black_is_one != 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageExportSelector {
    Reference { object: i32, generation: i32 },
    NthOfType { occurrence: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageExportRequest {
    pub page_index: u32,
    pub target_type: ImageExportType,
    pub selector: ImageExportSelector,
}

#[derive(Debug, Clone, Copy)]
pub struct PageInfo {
    pub page: u32,
    pub image_count: u32,
    pub object_count: u64,
}

#[derive(Debug, Clone)]
pub struct ImageCollection {
    pub images: Vec<ImageInfo>,
    pub pages: Vec<PageInfo>,
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
    pub dpi: (f64, f64),
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
            dpi: (value.dpi_x, value.dpi_y),
            xref,
        }
    }
}

impl From<SplashPageInfo> for PageInfo {
    fn from(value: SplashPageInfo) -> Self {
        Self {
            page: value.page_number,
            image_count: value.image_count,
            object_count: value.object_count,
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

pub(crate) fn get_poppler_version() -> (u32, u32, u32) {
    let mut version = MaybeUninit::<VersionInfo>::uninit();
    unsafe {
        splash_get_version(version.as_mut_ptr());
        let version = version.assume_init();
        (version.major, version.minor, version.patch)
    }
}
