#include <nanobind/nanobind.h>
#include <nanobind/ndarray.h>
#include <nanobind/stl/optional.h>
#include <nanobind/stl/string.h>
#include <nanobind/stl/tuple.h>
#include <nanobind/stl/vector.h>

#include <cmath>
#include <cstdint>
#include <cstring>
#include <optional>
#include <stdexcept>
#include <string>
#include <tuple>
#include <vector>

#include "exporter_bridge.h"
#include "splash_bridge.h"

namespace nb = nanobind;

namespace {

void check_poppler_error(int status, char *error)
{
    if (status == 0)
    {
        return;
    }
    std::string msg = error ? error : "unknown poppler error";
    if (error)
    {
        ntsplash_renderer_free_cstr(error);
    }
    throw std::runtime_error(msg);
}

enum class PyColorMode
{
    Mono1 = NTSPLASH_COLOR_MODE_MONO1,
    Mono8 = NTSPLASH_COLOR_MODE_MONO8,
    Rgb8 = NTSPLASH_COLOR_MODE_RGB8,
    Bgr8 = NTSPLASH_COLOR_MODE_BGR8,
    Xbgr8 = NTSPLASH_COLOR_MODE_XBGR8,
    Cmyk8 = NTSPLASH_COLOR_MODE_CMYK8,
    DeviceN8 = NTSPLASH_COLOR_MODE_DEVICEN8,
};

enum class PyImageColorSpace
{
    Unknown = NTSPLASH_IMAGE_COLORSPACE_UNKNOWN,
    DeviceGray = NTSPLASH_IMAGE_COLORSPACE_DEVICE_GRAY,
    DeviceRgb = NTSPLASH_IMAGE_COLORSPACE_DEVICE_RGB,
    DeviceCmyk = NTSPLASH_IMAGE_COLORSPACE_DEVICE_CMYK,
    Lab = NTSPLASH_IMAGE_COLORSPACE_LAB,
    Icc = NTSPLASH_IMAGE_COLORSPACE_ICC,
    Indexed = NTSPLASH_IMAGE_COLORSPACE_INDEXED,
    Pattern = NTSPLASH_IMAGE_COLORSPACE_PATTERN,
    Separation = NTSPLASH_IMAGE_COLORSPACE_SEPARATION,
    DeviceN = NTSPLASH_IMAGE_COLORSPACE_DEVICEN,
};

enum class PyImageType
{
    Unknown = NTSPLASH_IMAGE_TYPE_UNKNOWN,
    Image = NTSPLASH_IMAGE_TYPE_IMAGE,
    Stencil = NTSPLASH_IMAGE_TYPE_STENCIL,
    Mask = NTSPLASH_IMAGE_TYPE_MASK,
    SoftMask = NTSPLASH_IMAGE_TYPE_SOFT_MASK,
};

enum class PyCropMode
{
    Unknown = NTSPLASH_CROP_MODE_UNKNOWN,
    MediaBox = NTSPLASH_CROP_MODE_MEDIA_BOX,
    CropBox = NTSPLASH_CROP_MODE_CROP_BOX,
};

enum class PyZeroWidthLineMode
{
    Default = NTSPLASH_ZERO_WIDTH_LINE_DEFAULT,
    Hairline = NTSPLASH_ZERO_WIDTH_LINE_HAIRLINE,
    Nothing = NTSPLASH_ZERO_WIDTH_LINE_NOTHING,
};

enum class PyImageExportMatchMode
{
    ByRef = NTIMAGE_EXPORT_MATCH_BY_REF,
    ByOccurrence = NTIMAGE_EXPORT_MATCH_BY_OCCURRENCE,
};

enum class PyImageExportType
{
    Image = NTIMAGE_EXPORT_TYPE_IMAGE,
    Stencil = NTIMAGE_EXPORT_TYPE_STENCIL,
    Mask = NTIMAGE_EXPORT_TYPE_MASK,
    SoftMask = NTIMAGE_EXPORT_TYPE_SOFT_MASK,
};

enum class PyImageExportFormat
{
    Unknown = NTIMAGE_EXPORT_FORMAT_UNKNOWN,
    Rgb = NTIMAGE_EXPORT_FORMAT_RGB,
    Rgb48 = NTIMAGE_EXPORT_FORMAT_RGB48,
    Gray = NTIMAGE_EXPORT_FORMAT_GRAY,
    Monochrome = NTIMAGE_EXPORT_FORMAT_MONOCHROME,
    Cmyk = NTIMAGE_EXPORT_FORMAT_CMYK,
};

enum class PyImageExportExtension
{
    // Values must match the C++ internal NTImageExtension enum (and Rust's
    // ImageExportExtension), not the misleading nt_image_export_extension_t
    // labels in exporter_bridge.h which insert an extra CCITT_TIFF entry.
    Jpeg = 0,
    Jp2 = 1,
    Jbig2 = 2,
    Ccitt = 3,
    Png = 4,
    Tiff = 5,
    Pnm = 6,
};

struct XYZColor {
    double x = 0.0;
    double y = 0.0;
    double z = 0.0;
};

struct MinMaxRange {
    double min = 0.0;
    double max = 0.0;
};

// Forward declaration for recursion.
class ColorSpace;

class ColorSpace
{
public:
    PyImageColorSpace mode = PyImageColorSpace::Unknown;

    // Separation
    std::optional<std::string> separation_name;

    // Indexed
    uint32_t indexed_hival = 0;

    // DeviceN
    std::vector<std::string> devicen_names;

    // Lab
    std::optional<XYZColor> lab_white;
    std::optional<XYZColor> lab_black;
    std::optional<MinMaxRange> lab_a;
    std::optional<MinMaxRange> lab_b;

    // Recursive alternate (for Separation/ICC/Indexed/DeviceN)
    std::optional<nb::object> alternate;
};

nb::object build_colorspace(const void *handle)
{
    if (!handle)
    {
        return nb::none();
    }

    ColorSpace cs;
    cs.mode = static_cast<PyImageColorSpace>(ntgfxcs_get_color_mode(handle));

    switch (cs.mode)
    {
    case PyImageColorSpace::Indexed: {
        ntcolorspaces_indexed_info_t info{};
        if (ntgfxcs_get_indexed_info(handle, &info))
        {
            cs.indexed_hival = info.hival;
            cs.alternate = build_colorspace(info.base);
        }
        break;
    }
    case PyImageColorSpace::Separation: {
        ntcolorspaces_separation_info_t info{};
        if (ntgfxcs_get_separation_info(handle, &info))
        {
            cs.separation_name = info.name ? info.name : "";
            ntgfxcs_free_string(info.name);
            cs.alternate = build_colorspace(info.alternate);
        }
        break;
    }
    case PyImageColorSpace::DeviceN: {
        ntcolorspaces_devicen_info_t info{};
        if (ntgfxcs_get_devicen_info(handle, &info))
        {
            cs.devicen_names.reserve(info.count);
            for (uint32_t i = 0; i < info.count; ++i)
            {
                cs.devicen_names.emplace_back(info.names[i] ? info.names[i] : "");
            }
            ntgfxcs_free_string_array(info.names, info.count);
            cs.alternate = build_colorspace(info.alternate);
        }
        break;
    }
    case PyImageColorSpace::Lab: {
        ntcolorspaces_labxyz_info_t info{};
        if (ntgfxcs_get_labxyz_info(handle, &info))
        {
            cs.lab_white = XYZColor{info.whiteX, info.whiteY, info.whiteZ};
            cs.lab_black = XYZColor{info.blackX, info.blackY, info.blackZ};
            cs.lab_a = MinMaxRange{info.minA, info.maxA};
            cs.lab_b = MinMaxRange{info.minB, info.maxB};
        }
        break;
    }
    case PyImageColorSpace::Icc: {
        ntcolorspaces_icc_info_t info{};
        if (ntgfxcs_get_icc_info(handle, &info))
        {
            cs.alternate = build_colorspace(info.alternate);
        }
        break;
    }
    default:
        break;
    }

    return nb::cast(cs);
}

class ImageCollection;

class ImageInfo
{
public:
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t components = 0;
    uint32_t bits_per_component = 0;
    int32_t xref_object = -1;
    int32_t xref_generation = 0;
    uint32_t page_number = 0;
    double dpi_x = 0.0;
    double dpi_y = 0.0;
    PyImageType image_type = PyImageType::Unknown;
    PyImageColorSpace colorspace = PyImageColorSpace::Unknown;
    std::array<double, 6> ctm{};
    nb::object color_space;

    ImageInfo() = default;
    explicit ImageInfo(const ntsplash_image_info_t *info)
    {
        width = info->width;
        height = info->height;
        components = info->components;
        bits_per_component = info->bits_per_component;
        xref_object = info->xref_object;
        xref_generation = info->xref_generation;
        page_number = info->page_number;
        dpi_x = info->dpi_x;
        dpi_y = info->dpi_y;
        image_type = static_cast<PyImageType>(info->image_type);
        // Match Rust semantics: if the colorspace handle is missing, report Unknown.
        if (info->color_space_handle)
        {
            colorspace =
                static_cast<PyImageColorSpace>(ntgfxcs_get_color_mode(info->color_space_handle));
        }
        else
        {
            colorspace = PyImageColorSpace::Unknown;
        }
        std::memcpy(ctm.data(), info->ctm, sizeof(double) * 6);
        color_space = build_colorspace(info->color_space_handle);
    }

    std::optional<std::tuple<int32_t, int32_t>> xref() const
    {
        if (xref_object < 0)
        {
            return std::nullopt;
        }
        return std::make_tuple(xref_object, xref_generation);
    }

    std::tuple<double, double> dpi() const { return {dpi_x, dpi_y}; }
    std::tuple<double, double, double, double, double, double> matrix() const
    {
        return {ctm[0], ctm[1], ctm[2], ctm[3], ctm[4], ctm[5]};
    }
};

class PageInfo
{
public:
    uint32_t page_number = 0;
    uint32_t image_count = 0;
    uint64_t object_count = 0;
    bool is_pdf_a_compatible = false;
    std::array<double, 4> cropbox{};
    std::array<double, 4> mediabox{};
    nb::dict colorspaces;

    PageInfo() = default;
    explicit PageInfo(const ntsplash_page_info_t *info)
    {
        page_number = info->page_number;
        image_count = info->image_count;
        object_count = info->object_count;
        is_pdf_a_compatible = info->is_pdf_a_compatible != 0;
        std::memcpy(cropbox.data(), info->cropbox, sizeof(double) * 4);
        std::memcpy(mediabox.data(), info->mediabox, sizeof(double) * 4);

        colorspaces = nb::dict();
        for (uint32_t i = 0; i < info->colorspace_count; ++i)
        {
            const char *name = info->colorspaces[i].name;
            const void *handle = info->colorspaces[i].color_space_handle;
            if (name)
            {
                colorspaces[nb::str(name)] = build_colorspace(handle);
            }
        }
    }

    std::optional<std::tuple<double, double, double, double>> crop_box() const
    {
        if (cropbox[0] == 0.0 && cropbox[1] == 0.0 && cropbox[2] == 0.0 && cropbox[3] == 0.0)
        {
            return std::nullopt;
        }
        return std::make_tuple(cropbox[0], cropbox[1], cropbox[2], cropbox[3]);
    }

    std::optional<std::tuple<double, double, double, double>> media_box() const
    {
        if (mediabox[0] == 0.0 && mediabox[1] == 0.0 && mediabox[2] == 0.0 && mediabox[3] == 0.0)
        {
            return std::nullopt;
        }
        return std::make_tuple(mediabox[0], mediabox[1], mediabox[2], mediabox[3]);
    }
};

class ImageCollection
{
public:
    ntsplash_image_info_t *images_raw = nullptr;
    size_t image_len = 0;
    ntsplash_page_info_t *pages_raw = nullptr;
    size_t page_len = 0;
    std::vector<ImageInfo> images_vec;
    std::vector<PageInfo> pages_vec;

    ImageCollection(ntsplash_image_info_t *images, size_t image_len, ntsplash_page_info_t *pages,
                    size_t page_len)
        : images_raw(images), image_len(image_len), pages_raw(pages), page_len(page_len)
    {
        images_vec.reserve(image_len);
        for (size_t i = 0; i < image_len; ++i)
        {
            images_vec.emplace_back(&images[i]);
        }
        pages_vec.reserve(page_len);
        for (size_t i = 0; i < page_len; ++i)
        {
            pages_vec.emplace_back(&pages[i]);
        }
    }

    ~ImageCollection()
    {
        if (images_raw)
        {
            ntsplash_renderer_free_image_info(images_raw);
        }
        if (pages_raw)
        {
            ntsplash_renderer_free_page_info(pages_raw);
        }
    }

    const std::vector<ImageInfo> &images() const { return images_vec; }
    const std::vector<PageInfo> &pages() const { return pages_vec; }
};

class RenderedImage
{
public:
    nb::object data;
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t stride = 0;
    uint32_t components = 0;
    uint32_t bits_per_component = 0;
    PyColorMode color_mode = PyColorMode::Rgb8;

    explicit RenderedImage(ntsplash_image_t *image)
    {
        width = image->width;
        height = image->height;
        stride = image->stride;
        components = image->components;
        bits_per_component = image->bits_per_component;
        color_mode = static_cast<PyColorMode>(image->color_mode);

        if (image->data && image->len > 0)
        {
            data = copy_to_ndarray(image->data, image->len, image->width, image->height,
                                   image->stride, image->components,
                                   static_cast<ntsplash_color_mode_t>(image->color_mode));
        }
        else
        {
            data = nb::none();
        }

        ntsplash_renderer_free_image(image);
    }

private:
    static nb::object copy_to_ndarray(uint8_t *src, size_t len, uint32_t width, uint32_t height,
                                      uint32_t stride, uint32_t components,
                                      ntsplash_color_mode_t mode)
    {
        nb::ndarray<nb::numpy, uint8_t> arr;
        if (mode == NTSPLASH_COLOR_MODE_MONO1)
        {
            // Pack bits are returned as-is; shape is (height, stride).
            uint8_t *dst = new uint8_t[len];
            std::memcpy(dst, src, len);
            nb::capsule owner(dst, [](void *p) noexcept { delete[] static_cast<uint8_t *>(p); });
            arr = nb::ndarray<nb::numpy, uint8_t>(dst, {height, stride}, owner);
        }
        else if (components == 1)
        {
            // Grayscale: shape (height, width)
            const size_t row = static_cast<size_t>(width);
            uint8_t *dst = new uint8_t[row * height];
            for (uint32_t y = 0; y < height; ++y)
            {
                std::memcpy(dst + y * row, src + y * stride, row);
            }
            nb::capsule owner(dst, [](void *p) noexcept { delete[] static_cast<uint8_t *>(p); });
            arr = nb::ndarray<nb::numpy, uint8_t>(dst, {height, width}, owner);
        }
        else
        {
            // Multi-component: shape (height, width, components)
            const size_t row = static_cast<size_t>(width) * components;
            uint8_t *dst = new uint8_t[row * height];
            for (uint32_t y = 0; y < height; ++y)
            {
                std::memcpy(dst + y * row, src + y * stride, row);
            }
            nb::capsule owner(dst, [](void *p) noexcept { delete[] static_cast<uint8_t *>(p); });
            arr = nb::ndarray<nb::numpy, uint8_t>(dst, {height, width, components}, owner);
        }
        return nb::cast(arr);
    }
};

struct CcittParams {
    int32_t encoding = 0;
    int32_t columns = 0;
    int32_t rows = 0;
    int32_t damaged_rows_before_error = 0;
    bool end_of_line = false;
    bool byte_align = false;
    bool end_of_block = false;
    bool black_is_one = false;
};

class ExportedImage
{
public:
    nb::object data;
    std::optional<nb::object> jbig2_globals;
    std::optional<CcittParams> ccitt_params;
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t stride = 0;
    uint32_t components = 0;
    uint32_t bits_per_component = 0;
    double width_dpi = 0.0;
    double height_dpi = 0.0;
    PyImageExportFormat format = PyImageExportFormat::Unknown;
    PyImageExportType type = PyImageExportType::Image;
    PyImageExportExtension extension = PyImageExportExtension::Png;
    bool has_jbig2_globals = false;

    explicit ExportedImage(nt_image_export_image_t *image)
    {
        width = image->width;
        height = image->height;
        stride = image->stride;
        components = image->components;
        bits_per_component = image->bits_per_component;
        width_dpi = image->width_dpi;
        height_dpi = image->height_dpi;
        format = static_cast<PyImageExportFormat>(image->format);
        type = static_cast<PyImageExportType>(image->type);
        extension = static_cast<PyImageExportExtension>(image->extension);
        has_jbig2_globals = image->has_jbig2_globals != 0;

        if (image->data && image->len > 0)
        {
            data = nb::bytes(reinterpret_cast<const char *>(image->data), image->len);
        }
        else
        {
            data = nb::bytes("", 0);
        }

        if (image->has_jbig2_globals && image->jbig2_globals && image->jbig2_globals_len > 0)
        {
            jbig2_globals = nb::bytes(reinterpret_cast<const char *>(image->jbig2_globals),
                                      image->jbig2_globals_len);
        }
        else if (image->has_jbig2_globals)
        {
            jbig2_globals = nb::bytes("", 0);
        }

        if (image->has_ccitt_params)
        {
            CcittParams ccitt;
            ccitt.encoding = image->ccitt.encoding;
            ccitt.columns = image->ccitt.columns;
            ccitt.rows = image->ccitt.rows;
            ccitt.damaged_rows_before_error = image->ccitt.damaged_rows_before_error;
            ccitt.end_of_line = image->ccitt.end_of_line != 0;
            ccitt.byte_align = image->ccitt.byte_align != 0;
            ccitt.end_of_block = image->ccitt.end_of_block != 0;
            ccitt.black_is_one = image->ccitt.black_is_one != 0;
            ccitt_params = ccitt;
        }

        ntsplash_exporter_free(image);
    }
};

class Document
{
public:
    ntsplash_renderer_t *renderer = nullptr;
    std::string path;

    explicit Document(ntsplash_renderer_t *renderer, std::string path)
        : renderer(renderer), path(std::move(path))
    {
    }

    ~Document()
    {
        if (renderer)
        {
            ntsplash_renderer_destroy(renderer);
            renderer = nullptr;
        }
    }

    static Document *open(const std::string &path, const std::optional<std::string> &owner_password,
                          const std::optional<std::string> &user_password)
    {
        ntsplash_renderer_t *renderer = nullptr;
        char *error = nullptr;

        const char *owner_pw = owner_password ? owner_password->c_str() : nullptr;
        const char *user_pw = user_password ? user_password->c_str() : nullptr;

        int status = ntsplash_renderer_create(path.c_str(), owner_pw, user_pw, &renderer, &error);
        check_poppler_error(status, error);

        if (!renderer)
        {
            throw std::runtime_error("poppler returned a null renderer");
        }
        return new Document(renderer, path);
    }

    uint32_t page_count() const
    {
        uint32_t count = 0;
        char *error = nullptr;
        int status = ntsplash_renderer_page_count(renderer, &count, &error);
        check_poppler_error(status, error);
        return count;
    }

    RenderedImage render_page(uint32_t page_index, double dpi, PyColorMode color_mode,
                              PyCropMode crop_mode, PyZeroWidthLineMode zwl_mode) const
    {
        ntsplash_render_params_t params{};
        params.dpi = dpi;
        params.color_mode = static_cast<ntsplash_color_mode_t>(color_mode);
        params.crop_mode = static_cast<ntsplash_crop_mode_t>(crop_mode);
        params.zero_width_line_mode = static_cast<ntsplash_zero_width_line_mode_t>(zwl_mode);

        ntsplash_image_t image{};
        char *error = nullptr;
        int status = ntsplash_renderer_render_page(renderer, page_index, &params, &image, &error);
        check_poppler_error(status, error);

        return RenderedImage(&image);
    }

    ImageCollection *collect_images(uint32_t page_start, uint32_t page_end) const
    {
        ntsplash_image_info_t *images = nullptr;
        size_t image_len = 0;
        ntsplash_page_info_t *pages = nullptr;
        size_t page_len = 0;
        char *error = nullptr;

        int status = ntsplash_renderer_collect_images(renderer, &images, &image_len, &pages,
                                                      &page_len, page_start, page_end, &error);
        check_poppler_error(status, error);

        return new ImageCollection(images, image_len, pages, page_len);
    }

    ExportedImage export_image(uint32_t page_index, const std::optional<int32_t> &xref_object,
                               int32_t xref_generation, uint32_t occurrence_index,
                               PyImageExportType target_type, bool describe_only) const
    {
        nt_image_export_params_t params{};
        params.page_index = page_index;
        params.target_type = static_cast<nt_image_export_type_t>(target_type);

        if (xref_object && *xref_object >= 0)
        {
            params.match_mode = NTIMAGE_EXPORT_MATCH_BY_REF;
            params.xref_object = *xref_object;
            params.xref_generation = xref_generation;
            params.occurrence_index = 0;
        }
        else
        {
            params.match_mode = NTIMAGE_EXPORT_MATCH_BY_OCCURRENCE;
            params.xref_object = 0;
            params.xref_generation = 0;
            params.occurrence_index = occurrence_index;
        }

        nt_image_export_image_t image{};
        char *error = nullptr;
        int status =
            ntsplash_exporer_extract_page(renderer, &params, &image, describe_only, &error);
        check_poppler_error(status, error);

        return ExportedImage(&image);
    }
};

} // namespace

NB_MODULE(_core, m)
{
    m.doc() = "tiny-poppler core extension";

    // Enums
    nb::enum_<PyColorMode>(m, "ColorMode")
        .value("Mono1", PyColorMode::Mono1)
        .value("Mono8", PyColorMode::Mono8)
        .value("Rgb8", PyColorMode::Rgb8)
        .value("Bgr8", PyColorMode::Bgr8)
        .value("Xbgr8", PyColorMode::Xbgr8)
        .value("Cmyk8", PyColorMode::Cmyk8)
        .value("DeviceN8", PyColorMode::DeviceN8);

    nb::enum_<PyImageColorSpace>(m, "ImageColorSpace")
        .value("Unknown", PyImageColorSpace::Unknown)
        .value("DeviceGray", PyImageColorSpace::DeviceGray)
        .value("DeviceRgb", PyImageColorSpace::DeviceRgb)
        .value("DeviceCmyk", PyImageColorSpace::DeviceCmyk)
        .value("Lab", PyImageColorSpace::Lab)
        .value("Icc", PyImageColorSpace::Icc)
        .value("Indexed", PyImageColorSpace::Indexed)
        .value("Pattern", PyImageColorSpace::Pattern)
        .value("Separation", PyImageColorSpace::Separation)
        .value("DeviceN", PyImageColorSpace::DeviceN);

    nb::enum_<PyImageType>(m, "ImageType")
        .value("Unknown", PyImageType::Unknown)
        .value("Image", PyImageType::Image)
        .value("Stencil", PyImageType::Stencil)
        .value("Mask", PyImageType::Mask)
        .value("SoftMask", PyImageType::SoftMask);

    nb::enum_<PyCropMode>(m, "CropMode")
        .value("Unknown", PyCropMode::Unknown)
        .value("MediaBox", PyCropMode::MediaBox)
        .value("CropBox", PyCropMode::CropBox);

    nb::enum_<PyZeroWidthLineMode>(m, "ZeroWidthLineMode")
        .value("Default", PyZeroWidthLineMode::Default)
        .value("Hairline", PyZeroWidthLineMode::Hairline)
        .value("Nothing", PyZeroWidthLineMode::Nothing);

    nb::enum_<PyImageExportMatchMode>(m, "ImageExportMatchMode")
        .value("ByRef", PyImageExportMatchMode::ByRef)
        .value("ByOccurrence", PyImageExportMatchMode::ByOccurrence);

    nb::enum_<PyImageExportType>(m, "ImageExportType")
        .value("Image", PyImageExportType::Image)
        .value("Stencil", PyImageExportType::Stencil)
        .value("Mask", PyImageExportType::Mask)
        .value("SoftMask", PyImageExportType::SoftMask);

    nb::enum_<PyImageExportFormat>(m, "ImageExportFormat")
        .value("Unknown", PyImageExportFormat::Unknown)
        .value("Rgb", PyImageExportFormat::Rgb)
        .value("Rgb48", PyImageExportFormat::Rgb48)
        .value("Gray", PyImageExportFormat::Gray)
        .value("Monochrome", PyImageExportFormat::Monochrome)
        .value("Cmyk", PyImageExportFormat::Cmyk);

    nb::enum_<PyImageExportExtension>(m, "ImageExportExtension")
        .value("Jpeg", PyImageExportExtension::Jpeg)
        .value("Jp2", PyImageExportExtension::Jp2)
        .value("Jbig2", PyImageExportExtension::Jbig2)
        .value("Ccitt", PyImageExportExtension::Ccitt)
        .value("Png", PyImageExportExtension::Png)
        .value("Tiff", PyImageExportExtension::Tiff)
        .value("Pnm", PyImageExportExtension::Pnm);

    // Colorspace structs
    nb::class_<XYZColor>(m, "XYZColor")
        .def_ro("x", &XYZColor::x)
        .def_ro("y", &XYZColor::y)
        .def_ro("z", &XYZColor::z)
        .def("__repr__", [](const XYZColor &c) {
            return nb::str("XYZColor(x={}, y={}, z={})").format(c.x, c.y, c.z);
        });

    nb::class_<MinMaxRange>(m, "MinMaxRange")
        .def_ro("min", &MinMaxRange::min)
        .def_ro("max", &MinMaxRange::max)
        .def("__repr__", [](const MinMaxRange &r) {
            return nb::str("MinMaxRange(min={}, max={})").format(r.min, r.max);
        });

    nb::class_<ColorSpace>(m, "ColorSpace")
        .def_ro("mode", &ColorSpace::mode)
        .def_ro("separation_name", &ColorSpace::separation_name)
        .def_ro("indexed_hival", &ColorSpace::indexed_hival)
        .def_ro("devicen_names", &ColorSpace::devicen_names)
        .def_ro("lab_white", &ColorSpace::lab_white)
        .def_ro("lab_black", &ColorSpace::lab_black)
        .def_ro("lab_a", &ColorSpace::lab_a)
        .def_ro("lab_b", &ColorSpace::lab_b)
        .def_ro("alternate", &ColorSpace::alternate)
        .def("__repr__",
             [](const ColorSpace &cs) { return nb::str("ColorSpace(mode={})").format(cs.mode); });

    // ImageInfo
    nb::class_<ImageInfo>(m, "ImageInfo")
        .def_ro("width", &ImageInfo::width)
        .def_ro("height", &ImageInfo::height)
        .def_ro("components", &ImageInfo::components)
        .def_ro("bits_per_component", &ImageInfo::bits_per_component)
        .def_ro("page", &ImageInfo::page_number)
        .def_ro("image_type", &ImageInfo::image_type)
        .def_ro("colorspace", &ImageInfo::colorspace)
        .def_ro("color_space", &ImageInfo::color_space)
        .def("xref", &ImageInfo::xref)
        .def("dpi", &ImageInfo::dpi)
        .def("matrix", &ImageInfo::matrix)
        .def("__repr__", [](const ImageInfo &info) {
            return nb::str("ImageInfo(page={}, {}x{}, type={}, colorspace={})")
                .format(info.page_number, info.width, info.height, info.image_type,
                        info.colorspace);
        });

    // PageInfo
    nb::class_<PageInfo>(m, "PageInfo")
        .def_ro("page_number", &PageInfo::page_number)
        .def_ro("image_count", &PageInfo::image_count)
        .def_ro("object_count", &PageInfo::object_count)
        .def_ro("is_pdf_a_compatible", &PageInfo::is_pdf_a_compatible)
        .def_ro("colorspaces", &PageInfo::colorspaces)
        .def("crop_box", &PageInfo::crop_box)
        .def("media_box", &PageInfo::media_box)
        .def("__repr__", [](const PageInfo &p) {
            return nb::str("PageInfo(page={}, images={}, objects={})")
                .format(p.page_number, p.image_count, p.object_count);
        });

    // ImageCollection
    nb::class_<ImageCollection>(m, "ImageCollection")
        .def_ro("images", &ImageCollection::images_vec)
        .def_ro("pages", &ImageCollection::pages_vec)
        .def("__repr__", [](const ImageCollection &coll) {
            return nb::str("ImageCollection(images={}, pages={})")
                .format(coll.images_vec.size(), coll.pages_vec.size());
        });

    // RenderedImage
    nb::class_<RenderedImage>(m, "RenderedImage")
        .def_ro("data", &RenderedImage::data)
        .def_ro("width", &RenderedImage::width)
        .def_ro("height", &RenderedImage::height)
        .def_ro("stride", &RenderedImage::stride)
        .def_ro("components", &RenderedImage::components)
        .def_ro("bits_per_component", &RenderedImage::bits_per_component)
        .def_ro("color_mode", &RenderedImage::color_mode)
        .def("__repr__", [](const RenderedImage &img) {
            return nb::str("RenderedImage({}x{}, mode={}, {}bpc)")
                .format(img.width, img.height, img.color_mode, img.bits_per_component);
        });

    // CCITT params
    nb::class_<CcittParams>(m, "CcittParams")
        .def_ro("encoding", &CcittParams::encoding)
        .def_ro("columns", &CcittParams::columns)
        .def_ro("rows", &CcittParams::rows)
        .def_ro("damaged_rows_before_error", &CcittParams::damaged_rows_before_error)
        .def_ro("end_of_line", &CcittParams::end_of_line)
        .def_ro("byte_align", &CcittParams::byte_align)
        .def_ro("end_of_block", &CcittParams::end_of_block)
        .def_ro("black_is_one", &CcittParams::black_is_one)
        .def("__repr__", [](const CcittParams &p) {
            return nb::str("CcittParams(encoding={}, {}x{})").format(p.encoding, p.columns, p.rows);
        });

    // ExportedImage
    nb::class_<ExportedImage>(m, "ExportedImage")
        .def_prop_ro("data", [](const ExportedImage &img) { return img.data; })
        .def_prop_ro("jbig2_globals", [](const ExportedImage &img) { return img.jbig2_globals; })
        .def_ro("ccitt_params", &ExportedImage::ccitt_params)
        .def_ro("width", &ExportedImage::width)
        .def_ro("height", &ExportedImage::height)
        .def_ro("stride", &ExportedImage::stride)
        .def_ro("components", &ExportedImage::components)
        .def_ro("bits_per_component", &ExportedImage::bits_per_component)
        .def_ro("width_dpi", &ExportedImage::width_dpi)
        .def_ro("height_dpi", &ExportedImage::height_dpi)
        .def_ro("format", &ExportedImage::format)
        .def_ro("type", &ExportedImage::type)
        .def_ro("extension", &ExportedImage::extension)
        .def_ro("has_jbig2_globals", &ExportedImage::has_jbig2_globals)
        .def("__repr__", [](const ExportedImage &img) {
            return nb::str("ExportedImage({}x{}, format={}, ext={})")
                .format(img.width, img.height, img.format, img.extension);
        });

    // Document
    nb::class_<Document>(m, "Document")
        .def_static("open", &Document::open, nb::arg("path"),
                    nb::arg("owner_password") = std::nullopt,
                    nb::arg("user_password") = std::nullopt)
        .def_prop_ro("page_count", &Document::page_count)
        .def("render_page", &Document::render_page, nb::arg("page_index"), nb::arg("dpi") = 150.0,
             nb::arg("color_mode") = PyColorMode::Rgb8, nb::arg("crop_mode") = PyCropMode::CropBox,
             nb::arg("zero_width_line_mode") = PyZeroWidthLineMode::Default)
        .def("collect_images", &Document::collect_images, nb::arg("page_start") = 1,
             nb::arg("page_end") = 0)
        .def("export_image", &Document::export_image, nb::arg("page_index"),
             nb::arg("xref_object") = std::nullopt, nb::arg("xref_generation") = 0,
             nb::arg("occurrence_index") = 0, nb::arg("target_type") = PyImageExportType::Image,
             nb::arg("describe_only") = false)
        .def("__repr__", [](const Document &doc) {
            uint32_t count = 0;
            if (doc.renderer)
            {
                char *error = nullptr;
                ntsplash_renderer_page_count(doc.renderer, &count, &error);
                if (error)
                {
                    ntsplash_renderer_free_cstr(error);
                }
            }
            return nb::str("Document(path='{}', pages={})").format(doc.path, count);
        });

    // Version
    m.def("get_version", []() {
        ntsplash_version_t version{};
        ntsplash_get_version(&version);
        return std::make_tuple(version.major, version.minor, version.patch);
    });
}
