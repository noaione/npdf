#include "splash_bridge.h"

#include <cmath>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <mutex>
#include <new>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include "GfxState.h"
#include "GlobalParams.h"
#include "OutputDev.h"
#include "PDFDoc.h"
#include "SplashOutputDev.h"
#include "Stream.h"
#include "goo/GooString.h"
#include "poppler-config.h"
#include "splash/SplashBitmap.h"
#include "splash/SplashTypes.h"
#include "splash_renderer_internal.h"

#define VERSION_MAJOR POPP

namespace {
constexpr int kNTSplashBitmapRowPad = 4;
constexpr bool kNTSplashReverseVideo = false;
constexpr bool kNTSplashTopDownBitmap = true;
constexpr SplashThinLineMode kNTSplashThinLineMode = splashThinLineDefault;

// This flag handles the synchronization state
static std::once_flag kTSplashInitFlag;

std::optional<SplashColorMode> ntsplash_upconvert_color_mode(ntsplash_color_mode_t mode)
{
    switch (mode)
    {
    case NTSPLASH_COLOR_MODE_MONO1:
        return splashModeMono1;
    case NTSPLASH_COLOR_MODE_MONO8:
        return splashModeMono8;
    case NTSPLASH_COLOR_MODE_RGB8:
        return splashModeRGB8;
    case NTSPLASH_COLOR_MODE_BGR8:
        return splashModeBGR8;
    case NTSPLASH_COLOR_MODE_XBGR8:
        return splashModeXBGR8;
    case NTSPLASH_COLOR_MODE_CMYK8:
        return splashModeCMYK8;
    case NTSPLASH_COLOR_MODE_DEVICEN8:
        return splashModeDeviceN8;
    default:
        return std::nullopt;
    }
}

std::optional<SplashZeroWidthLineMode>
ntsplash_upconvert_zwl_mode(ntsplash_zero_width_line_mode_t mode)
{
    switch (mode)
    {
    case NTSPLASH_ZERO_WIDTH_LINE_DEFAULT:
        return splashZeroWidthLineDefault;
    case NTSPLASH_ZERO_WIDTH_LINE_HAIRLINE:
        return splashZeroWidthLineHairline;
    case NTSPLASH_ZERO_WIDTH_LINE_NOTHING:
        return splashZeroWidthLineNothing;
    default:
        return std::nullopt;
    }
}

void ntsplash_ensure_global_params()
{
    std::call_once(kTSplashInitFlag, []() {
        globalParams = std::make_unique<GlobalParams>();
        globalParams->setErrQuiet(true);
    });
}

ntsplash_image_colorspace_t ntsplash_upconvert_colorspace(const GfxColorSpace *color_space)
{
    if (!color_space)
    {
        return NTSPLASH_IMAGE_COLORSPACE_UNKNOWN;
    }

    switch (color_space->getMode())
    {
    case GfxColorSpaceMode::csDeviceGray:
    case GfxColorSpaceMode::csCalGray:
        return NTSPLASH_IMAGE_COLORSPACE_DEVICE_GRAY;
    case GfxColorSpaceMode::csDeviceRGB:
    case GfxColorSpaceMode::csCalRGB:
        return NTSPLASH_IMAGE_COLORSPACE_DEVICE_RGB;
    case GfxColorSpaceMode::csDeviceCMYK:
        return NTSPLASH_IMAGE_COLORSPACE_DEVICE_CMYK;
    case GfxColorSpaceMode::csLab:
        return NTSPLASH_IMAGE_COLORSPACE_LAB;
    case GfxColorSpaceMode::csICCBased:
        return NTSPLASH_IMAGE_COLORSPACE_ICC;
    case GfxColorSpaceMode::csIndexed:
        return NTSPLASH_IMAGE_COLORSPACE_INDEXED;
    case GfxColorSpaceMode::csPattern:
        return NTSPLASH_IMAGE_COLORSPACE_PATTERN;
    case GfxColorSpaceMode::csSeparation:
        return NTSPLASH_IMAGE_COLORSPACE_SEPARATION;
    case GfxColorSpaceMode::csDeviceN:
        return NTSPLASH_IMAGE_COLORSPACE_DEVICEN;
    default:
        return NTSPLASH_IMAGE_COLORSPACE_UNKNOWN;
    }
}

bool ntsplash_copy_bitmap(SplashBitmap *bitmap, SplashColorMode mode, ntsplash_image_t *out_image,
                          char **error_out)
{
    if (!bitmap || !out_image)
    {
        ntsplash_set_error(error_out, "internal splash renderer error");
        return false;
    }

    const int width = bitmap->getWidth();
    const int height = bitmap->getHeight();
    const int row_size = bitmap->getRowSize();

    if (width <= 0 || height <= 0 || row_size <= 0)
    {
        ntsplash_set_error(error_out, "received empty bitmap from renderer");
        return false;
    }

    const size_t stride = static_cast<size_t>(row_size);
    const size_t total_size = stride * static_cast<size_t>(height);

    auto *buffer = static_cast<uint8_t *>(std::malloc(total_size));
    if (!buffer)
    {
        ntsplash_set_error(error_out, "unable to allocate buffer for rendered page");
        return false;
    }

    std::memcpy(buffer, bitmap->getDataPtr(), total_size);

    const SplashColorMode bitmap_mode = bitmap->getMode();
    if (bitmap_mode != mode)
    {
        std::free(buffer);
        ntsplash_set_error(error_out, "renderer returned bitmap with unexpected color mode");
        return false;
    }
    const int components = splashColorModeNComps[static_cast<int>(bitmap_mode)];
    const uint32_t bits_per_component = bitmap_mode == splashModeMono1 ? 1u : 8u;

    out_image->data = buffer;
    out_image->len = total_size;
    out_image->width = static_cast<uint32_t>(width);
    out_image->height = static_cast<uint32_t>(height);
    out_image->stride = static_cast<uint32_t>(stride);
    out_image->components = static_cast<uint32_t>(components);
    out_image->color_mode = static_cast<ntsplash_color_mode_t>(bitmap_mode);
    out_image->bits_per_component = bits_per_component;

    return true;
}

struct NTSplashCollectedImage {
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t components = 0;
    uint32_t bits_per_component = 0;
    int32_t xref_object = -1;
    int32_t xref_generation = 0;
    uint32_t page_number = 0;
    double_t dpi_x = 0;
    double_t dpi_y = 0;
    ntsplash_image_type_t image_type = NTSPLASH_IMAGE_TYPE_UNKNOWN;
    ntsplash_image_colorspace_t colorspace = NTSPLASH_IMAGE_COLORSPACE_UNKNOWN;
    double ctm[6] = {1.0, 0.0, 0.0, 1.0, 0.0, 0.0}; // Default is identity matrix
    const void *color_space_handle = nullptr;
};

struct NTSplashCollectedPage {
    uint32_t page_number = 0;
    uint32_t image_count = 0;
    uint64_t object_count = 0;
    bool is_pdf_a_compatible = true;

    // If all 0's, then not set
    double cropbox[4] = {0.0, 0.0, 0.0, 0.0};
    double mediabox[4] = {0.0, 0.0, 0.0, 0.0};

    ntsplash_page_colorspace_entry_t *colorspaces = nullptr;
    uint32_t colorspace_count = 0;
};

const void *ntsplash_copy_color_space(const GfxColorSpace *space)
{
    if (!space)
    {
        return nullptr;
    }

    std::unique_ptr<GfxColorSpace> copy = space->copy();
    return static_cast<const void *>(copy.release());
}

class NTSplashImageCollector final : public OutputDev
{
public:
    explicit NTSplashImageCollector(std::vector<NTSplashCollectedImage> *images) : images_(images)
    {
    }

    bool upsideDown() override { return false; }
    bool useDrawChar() override { return false; }
    bool interpretType3Chars() override { return false; }

    void reset_for_page(Page *page, uint32_t page_number)
    {
        cur_page = page;
        cur_page_idx = page_number;
        total_objects_ = 0;          // reset object count for new page
        is_pdf_a_compatible_ = true; // we start assuming true for each page
    }

    uint64_t get_total_objects() const { return total_objects_; }
    bool is_pdf_a_compatible() const { return is_pdf_a_compatible_; }

    void drawImage(GfxState *state, Object *ref, Stream *str, int width, int height,
                   GfxImageColorMap *color_map, bool interpolate, const int *maskColors,
                   bool inlineImg) override
    {
        (void)state;
        (void)str;
        (void)maskColors;
        (void)inlineImg;
        (void)interpolate;
        total_objects_++;
        add_image(width, height, color_map, ref, state, NTSPLASH_IMAGE_TYPE_IMAGE);
    }

    void drawImageMask(GfxState *state, Object *ref, Stream *str, int width, int height,
                       bool invert, bool interpolate, bool inlineImg) override
    {
        (void)state;
        (void)str;
        (void)invert;
        (void)interpolate;
        (void)inlineImg;
        total_objects_++;
        add_mask(width, height, ref, state);
        is_pdf_a_compatible_ = false; // stencil is technically not allowed
    }

    void drawMaskedImage(GfxState *state, Object *ref, Stream *str, int width, int height,
                         GfxImageColorMap *color_map, bool interpolate, Stream *maskStr,
                         int maskWidth, int maskHeight, bool maskInvert,
                         bool maskInterpolate) override
    {
        (void)state;
        (void)str;
        (void)maskStr;
        (void)maskWidth;
        (void)maskHeight;
        (void)maskInvert;
        (void)maskInterpolate;
        (void)interpolate;
        total_objects_++;
        add_image(width, height, color_map, ref, state, NTSPLASH_IMAGE_TYPE_IMAGE);
        add_image(maskWidth, maskHeight, nullptr, ref, state, NTSPLASH_IMAGE_TYPE_MASK);
        is_pdf_a_compatible_ =
            false; // since a masked image was drawn, this page cannot be PDF/A compliant
    }

    void drawSoftMaskedImage(GfxState *state, Object *ref, Stream *str, int width, int height,
                             GfxImageColorMap *color_map, bool interpolate, Stream *maskStr,
                             int maskWidth, int maskHeight, GfxImageColorMap *maskColorMap,
                             bool maskInterpolate) override
    {
        (void)state;
        (void)str;
        (void)maskStr;
        (void)maskWidth;
        (void)maskHeight;
        (void)maskColorMap;
        (void)maskInterpolate;
        (void)interpolate;
        total_objects_++;
        add_image(width, height, color_map, ref, state, NTSPLASH_IMAGE_TYPE_IMAGE);
        add_image(maskWidth, maskHeight, maskColorMap, ref, state, NTSPLASH_IMAGE_TYPE_SOFT_MASK);
        is_pdf_a_compatible_ =
            false; // since a soft mask was drawn, this page cannot be PDF/A compliant
    }

    void drawString(GfxState *state, const GooString *s) override
    {
        (void)state;
        (void)s;
        total_objects_++;
        is_pdf_a_compatible_ = false; // since text was drawn, this page cannot be PDF/A compliant
    }

    void drawForm(Ref id) override
    {
        (void)id;
        total_objects_++;
        is_pdf_a_compatible_ = false; // since a form was drawn, this page cannot be PDF/A compliant
    }

    void stroke(GfxState *state) override
    {
        (void)state;
        total_objects_++;
        is_pdf_a_compatible_ =
            false; // since a stroke was drawn, this page cannot be PDF/A compliant
    }

    void fill(GfxState *state) override
    {
        (void)state;
        total_objects_++;
        is_pdf_a_compatible_ = false; // since a fill was drawn, this page cannot be PDF/A compliant
    }

    void eoFill(GfxState *state) override
    {
        (void)state;
        total_objects_++;
        is_pdf_a_compatible_ =
            false; // since an even-odd fill was drawn, this page cannot be PDF/A compliant
    }

    void clip(GfxState *state) override
    {
        (void)state;

        // Check clip
        bool is_clip_empty = state->isPath();
        if (!is_clip_empty)
        {
            total_objects_++;
            is_pdf_a_compatible_ =
                false; // since a clip was drawn, this page cannot be PDF/A compliant
        }
    }

    void eoClip(GfxState *state) override
    {
        (void)state;
        bool is_clip_empty = state->isPath();
        if (!is_clip_empty)
        {
            total_objects_++;
            is_pdf_a_compatible_ =
                false; // since an even-odd clip was drawn, this page cannot be PDF/A compliant
        }
    }

    void psXObject(Stream *psStream, Stream *level1Stream) override
    {
        (void)psStream;
        (void)level1Stream;
        total_objects_++;
        is_pdf_a_compatible_ =
            false; // since a PostScript XObject was drawn, this page cannot be PDF/A compliant
    }

    // Extract page colorspace dictionary (/Resources/ColorSpace).
    //
    // Returns a list of (resource name -> parsed colorspace).
    std::vector<std::pair<std::string, std::unique_ptr<GfxColorSpace>>> extractPageColorspaces()
    {
        std::vector<std::pair<std::string, std::unique_ptr<GfxColorSpace>>> result;

        if (!cur_page)
        {
            return result;
        }

        Dict *res_dict = cur_page->getResourceDict();
        if (!res_dict)
        {
            return result;
        }

        Object cs_obj = res_dict->lookup("ColorSpace");
        if (!cs_obj.isDict())
        {
            return result;
        }

        // get the colorspace dictionary
        Dict *cs_dict = cs_obj.getDict();

        // make dummy state for parsing
        const PDFRectangle *rect = cur_page->getMediaBox();
        std::unique_ptr<GfxState> state(new GfxState(72.0, 72.0, rect, 0, this->upsideDown()));

        for (int i = 0; i < cs_dict->getLength(); ++i)
        {
            const char *cs_name = cs_dict->getKey(i);
            // get value for the key
            Object cs_value = cs_dict->getVal(i);
            if (cs_value.isNull())
            {
                continue;
            }

            std::unique_ptr<GfxColorSpace> color_space =
                GfxColorSpace::parse(nullptr, &cs_value, this, state.get());
            if (!color_space)
            {
                continue;
            }

            result.emplace_back(cs_name ? std::string(cs_name) : std::string(),
                                std::move(color_space));
        }

        return result;
    }

private:
    void add_image(int width, int height, GfxImageColorMap *color_map, Object *ref, GfxState *state,
                   ntsplash_image_type_t image_type)
    {
        if (!images_)
        {
            return;
        }

        NTSplashCollectedImage info;
        info.page_number = cur_page_idx;
        if (width > 0)
        {
            info.width = static_cast<uint32_t>(width);
        }
        if (height > 0)
        {
            info.height = static_cast<uint32_t>(height);
        }
        info.image_type = image_type;

        if (color_map)
        {
            info.components = static_cast<uint32_t>(color_map->getNumPixelComps());
            info.bits_per_component = static_cast<uint32_t>(color_map->getBits());
            const GfxColorSpace *space = color_map->getColorSpace();
            info.colorspace = ntsplash_upconvert_colorspace(space);
            info.color_space_handle = ntsplash_copy_color_space(space);
        }
        else
        {
            info.components = 1;
            info.bits_per_component = 1;
            info.colorspace = NTSPLASH_IMAGE_COLORSPACE_DEVICE_GRAY;
            info.color_space_handle = nullptr;
        }

        if (ref && ref->isRef())
        {
            const auto reference = ref->getRef();
            info.xref_object = static_cast<int32_t>(reference.num);
            info.xref_generation = static_cast<int32_t>(reference.gen);
        }

        if (state)
        {
            std::pair<double, double> dpi = calculate_image_dpi(state->getCTM(), width, height);
            info.dpi_x = static_cast<double_t>(dpi.first);
            info.dpi_y = static_cast<double_t>(dpi.second);

            const double *ctm = state->getCTM();
            if (ctm)
            {
                std::memcpy(info.ctm, ctm, 6 * sizeof(double));
            }
        }

        images_->push_back(info);
    }

    void add_mask(int width, int height, Object *ref, GfxState *state)
    {
        if (!images_)
        {
            return;
        }

        NTSplashCollectedImage info;
        info.page_number = cur_page_idx;
        info.image_type = NTSPLASH_IMAGE_TYPE_STENCIL;
        if (width > 0)
        {
            info.width = static_cast<uint32_t>(width);
        }
        if (height > 0)
        {
            info.height = static_cast<uint32_t>(height);
        }
        info.components = 1;
        info.bits_per_component = 1;
        info.colorspace = NTSPLASH_IMAGE_COLORSPACE_DEVICE_GRAY;
        info.color_space_handle = nullptr;
        if (ref && ref->isRef())
        {
            const auto reference = ref->getRef();
            info.xref_object = static_cast<int32_t>(reference.num);
            info.xref_generation = static_cast<int32_t>(reference.gen);
        }
        if (state)
        {
            std::pair<double, double> dpi = calculate_image_dpi(state->getCTM(), width, height);
            info.dpi_x = static_cast<double_t>(dpi.first);
            info.dpi_y = static_cast<double_t>(dpi.second);
            const double *ctm = state->getCTM();
            if (ctm)
            {
                std::memcpy(info.ctm, ctm, 6 * sizeof(double));
            }
        }
        images_->push_back(info);
    }

    std::pair<double, double> calculate_image_dpi(const double *ctm, int width, int height)
    {
        if (!ctm)
        {
            return {0.0, 0.0};
        }

        // Calculate the scaling factors from the CTM
        double width2 = sqrt(ctm[0] * ctm[0] + ctm[1] * ctm[1]);
        double height2 = sqrt(ctm[2] * ctm[2] + ctm[3] * ctm[3]);

        double xppi = fabs(width * 72.0 / width2);
        double yppi = fabs(height * 72.0 / height2);

        return {xppi, yppi};
    }

    std::vector<NTSplashCollectedImage> *images_ = nullptr;
    Page *cur_page = nullptr;
    uint32_t cur_page_idx = 0;
    uint64_t total_objects_ = 0;
    bool is_pdf_a_compatible_ = false;
};

} // namespace

int ntsplash_renderer_create(const char *path, const char *owner_password,
                             const char *user_password, ntsplash_renderer_t **out_renderer,
                             char **error_out)
{
    if (!path || !out_renderer)
    {
        ntsplash_set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    ntsplash_ensure_global_params();

    auto goo_path = std::make_unique<GooString>(path);
    std::optional<GooString> owner_pw;
    if (owner_password != nullptr)
    {
        owner_pw.emplace(owner_password);
    }

    std::optional<GooString> user_pw;
    if (user_password != nullptr)
    {
        user_pw.emplace(user_password);
    }

    std::unique_ptr<PDFDoc> doc = std::make_unique<PDFDoc>(std::move(goo_path), owner_pw, user_pw);

    if (!doc->isOk())
    {
        const int error_code = doc->getErrorCode();
        ntsplash_set_error(error_out, ntsplash_stringify_error_code(error_code));
        return error_code == 0 ? errInternal : error_code;
    }

    auto renderer = std::make_unique<ntsplash_renderer>();
    renderer->doc = std::move(doc);

    *out_renderer = renderer.release();
    return errNone;
}

void ntsplash_renderer_destroy(ntsplash_renderer_t *renderer)
{
    if (!renderer)
    {
        return;
    }
    delete renderer;
}

int ntsplash_renderer_page_count(ntsplash_renderer_t *renderer, uint32_t *out_count,
                                 char **error_out)
{
    if (!renderer || !out_count)
    {
        ntsplash_set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    const int count = renderer->doc->getNumPages();
    if (count < 0)
    {
        ntsplash_set_error(error_out, "failed to query page count");
        return errInternal;
    }

    *out_count = static_cast<uint32_t>(count);
    return errNone;
}

int ntsplash_renderer_render_page(ntsplash_renderer_t *renderer, uint32_t page_index,
                                  const ntsplash_render_params_t *params,
                                  ntsplash_image_t *out_image, char **error_out)
{
    if (!renderer || !out_image)
    {
        ntsplash_set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    const int page_number = static_cast<int>(page_index) + 1;
    const int total_pages = renderer->doc->getNumPages();
    if (page_number < 1 || page_number > total_pages)
    {
        ntsplash_set_error(error_out, "page index out of range");
        return errBadPageNum;
    }

    if (!params)
    {
        ntsplash_set_error(error_out, "render parameters cannot be null");
        return errInternal;
    }

    auto maybe_mode = ntsplash_upconvert_color_mode(params->color_mode);
    if (!maybe_mode)
    {
        ntsplash_set_error(error_out, "unsupported Splash color mode requested");
        return errInternal;
    }

    auto maybe_zero_width_line_mode = ntsplash_upconvert_zwl_mode(params->zero_width_line_mode);
    if (!maybe_zero_width_line_mode)
    {
        ntsplash_set_error(error_out, "unsupported Splash zero-width line mode requested");
        return errInternal;
    }

    if (!params->dpi || params->dpi <= 0.0)
    {
        ntsplash_set_error(error_out, "DPI must be a positive number");
        return errInternal;
    }

    if (!params->crop_mode)
    {
        ntsplash_set_error(error_out, "invalid crop mode specified");
        return errInternal;
    }

    Page *page = renderer->doc->getPage(page_number); // Preload page to set up crop boxes, etc.

    const double clamped_dpi = params->dpi > 0.0 ? params->dpi : 72.0;
    bool use_media_box = params->crop_mode == NTSPLASH_CROP_MODE_MEDIA_BOX;

    const SplashColorMode requested_mode = *maybe_mode;
    const SplashZeroWidthLineMode requested_zero_width_line_mode = *maybe_zero_width_line_mode;
    const bool enable_overprint =
        requested_mode == splashModeCMYK8 || requested_mode == splashModeDeviceN8;

    SplashColor paper_color;
    if (enable_overprint)
    {
        splashClearColor(paper_color);
    }
    else
    {
        paper_color[0] = 255;
        paper_color[1] = 255;
        paper_color[2] = 255;
        // paper_color[3] = 255;
    }

    SplashOutputDev output_dev(requested_mode, kNTSplashBitmapRowPad, kNTSplashReverseVideo,
                               paper_color, kNTSplashTopDownBitmap, kNTSplashThinLineMode,
                               enable_overprint);
    output_dev.setVectorAntialias(true);
    output_dev.setFontAntialias(true);
    output_dev.setEnableFreeType(true);
    output_dev.setFreeTypeHinting(true, true);
    output_dev.setZeroWidthLineMode(requested_zero_width_line_mode);
    output_dev.startDoc(renderer->doc.get());

    page->display(&output_dev, clamped_dpi, clamped_dpi, 0, use_media_box, false, false);

    std::unique_ptr<SplashBitmap> bitmap(output_dev.takeBitmap());
    if (!bitmap)
    {
        ntsplash_set_error(error_out, "renderer produced no bitmap");
        return errInternal;
    }

    if (!ntsplash_copy_bitmap(bitmap.get(), requested_mode, out_image, error_out))
    {
        return errInternal;
    }

    return errNone;
}

int ntsplash_renderer_collect_images(ntsplash_renderer_t *renderer,
                                     ntsplash_image_info_t **out_images, size_t *out_image_len,
                                     ntsplash_page_info_t **out_pages, size_t *out_page_len,
                                     uint32_t page_start, uint32_t page_end, char **error_out)
{
    if (!renderer || !out_images || !out_image_len || !out_pages || !out_page_len)
    {
        ntsplash_set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    *out_images = nullptr;
    *out_image_len = 0;
    *out_pages = nullptr;
    *out_page_len = 0;

    const int total_pages = renderer->doc->getNumPages();
    if (total_pages <= 0)
    {
        return errNone;
    }

    uint32_t start_page = page_start > 0 ? page_start : 1;
    uint32_t end_page = page_end > 0 ? page_end : static_cast<uint32_t>(total_pages);
    if (start_page < 1 || start_page > static_cast<uint32_t>(total_pages))
    {
        ntsplash_set_error(error_out, "start page out of range");
        return errBadPageNum;
    }
    if (end_page < start_page || end_page > static_cast<uint32_t>(total_pages))
    {
        ntsplash_set_error(error_out, "end page out of range");
        return errBadPageNum;
    }

    std::vector<NTSplashCollectedImage> collected;
    collected.reserve(static_cast<size_t>(total_pages));

    const uint32_t page_span = end_page - start_page + 1;
    std::vector<NTSplashCollectedPage> page_summaries;
    page_summaries.reserve(static_cast<size_t>(page_span));

    NTSplashImageCollector collector(&collected);

    for (uint32_t page_number = start_page; page_number <= end_page; ++page_number)
    {
        const size_t before = collected.size();
        Page *page = renderer->doc->getPage(static_cast<int>(page_number));
        collector.reset_for_page(page, page_number);
        page->display(&collector, 72.0, 72.0, 0, true, true, false);
        const size_t after = collected.size();

        const PDFRectangle *cropbox = page->getCropBox();
        const PDFRectangle *mediabox = page->getMediaBox();

        NTSplashCollectedPage summary;
        summary.page_number = page_number;
        summary.image_count = static_cast<uint32_t>(after - before);
        summary.object_count = collector.get_total_objects();
        summary.is_pdf_a_compatible = collector.is_pdf_a_compatible();

        // Collect page /Resources/ColorSpace dictionary into owned entries.
        auto page_colorspaces = collector.extractPageColorspaces();
        if (!page_colorspaces.empty())
        {
            const size_t allocation_count = page_colorspaces.size();
            const size_t header_size = sizeof(size_t);
            const size_t payload_size = allocation_count * sizeof(ntsplash_page_colorspace_entry_t);
            void *raw = std::malloc(header_size + payload_size);
            if (!raw)
            {
                // Cleanup any previously allocated page colorspaces.
                for (auto &existing : page_summaries)
                {
                    if (existing.colorspaces)
                    {
                        ntsplash_renderer_free_page_colorspaces(existing.colorspaces);
                        existing.colorspaces = nullptr;
                        existing.colorspace_count = 0;
                    }
                }
                ntsplash_set_error(error_out, "unable to allocate colorspace dictionary buffer");
                return errInternal;
            }

            auto *header = static_cast<size_t *>(raw);
            *header = allocation_count;
            auto *entries = reinterpret_cast<ntsplash_page_colorspace_entry_t *>(header + 1);
            for (size_t i = 0; i < allocation_count; ++i)
            {
                entries[i].name = nullptr;
                entries[i].color_space_handle = nullptr;
            }

            bool ok = true;
            for (size_t i = 0; i < allocation_count; ++i)
            {
                entries[i].name = strdup(page_colorspaces[i].first.c_str());
                entries[i].color_space_handle =
                    static_cast<const void *>(page_colorspaces[i].second.release());
                if (!entries[i].name || !entries[i].color_space_handle)
                {
                    ok = false;
                    break;
                }
            }

            if (!ok)
            {
                ntsplash_renderer_free_page_colorspaces(entries);
                for (auto &existing : page_summaries)
                {
                    if (existing.colorspaces)
                    {
                        ntsplash_renderer_free_page_colorspaces(existing.colorspaces);
                        existing.colorspaces = nullptr;
                        existing.colorspace_count = 0;
                    }
                }
                ntsplash_set_error(error_out, "unable to allocate colorspace dictionary entry");
                return errInternal;
            }

            summary.colorspaces = entries;
            summary.colorspace_count = static_cast<uint32_t>(allocation_count);
        }

        if (cropbox)
        {
            summary.cropbox[0] = cropbox->x1;
            summary.cropbox[1] = cropbox->y1;
            summary.cropbox[2] = cropbox->x2;
            summary.cropbox[3] = cropbox->y2;
        }
        if (mediabox)
        {
            summary.mediabox[0] = mediabox->x1;
            summary.mediabox[1] = mediabox->y1;
            summary.mediabox[2] = mediabox->x2;
            summary.mediabox[3] = mediabox->y2;
        }
        page_summaries.push_back(summary);
    }

    ntsplash_image_info_t *image_buffer = nullptr;
    if (!collected.empty())
    {
        const size_t allocation_count = collected.size();
        const size_t header_size = sizeof(size_t);
        const size_t payload_size = allocation_count * sizeof(ntsplash_image_info_t);
        void *raw = std::malloc(header_size + payload_size);
        if (!raw)
        {
            ntsplash_set_error(error_out, "unable to allocate image metadata buffer");
            return errInternal;
        }

        auto *header = static_cast<size_t *>(raw);
        *header = allocation_count;

        image_buffer = reinterpret_cast<ntsplash_image_info_t *>(header + 1);
        if (!image_buffer)
        {
            std::free(raw);
            ntsplash_set_error(error_out, "unable to allocate image metadata buffer");
            return errInternal;
        }

        for (size_t i = 0; i < collected.size(); ++i)
        {
            image_buffer[i].width = collected[i].width;
            image_buffer[i].height = collected[i].height;
            image_buffer[i].dpi_x = collected[i].dpi_x;
            image_buffer[i].dpi_y = collected[i].dpi_y;
            image_buffer[i].components = collected[i].components;
            image_buffer[i].bits_per_component = collected[i].bits_per_component;
            image_buffer[i].xref_object = collected[i].xref_object;
            image_buffer[i].xref_generation = collected[i].xref_generation;
            image_buffer[i].image_type = collected[i].image_type;
            image_buffer[i].colorspace = collected[i].colorspace;
            image_buffer[i].color_space_handle = collected[i].color_space_handle;
            image_buffer[i].page_number = collected[i].page_number;
            for (int j = 0; j < 6; ++j)
            {
                image_buffer[i].ctm[j] = collected[i].ctm[j];
            }
        }
    }

    ntsplash_page_info_t *page_buffer = nullptr;
    if (!page_summaries.empty())
    {
        const size_t allocation_count = page_summaries.size();
        const size_t header_size = sizeof(size_t);
        const size_t payload_size = allocation_count * sizeof(ntsplash_page_info_t);
        void *raw = std::malloc(header_size + payload_size);
        if (!raw)
        {
            if (image_buffer)
            {
                ntsplash_renderer_free_image_info(image_buffer);
            }
            for (auto &existing : page_summaries)
            {
                if (existing.colorspaces)
                {
                    ntsplash_renderer_free_page_colorspaces(existing.colorspaces);
                    existing.colorspaces = nullptr;
                    existing.colorspace_count = 0;
                }
            }
            ntsplash_set_error(error_out, "unable to allocate page metadata buffer");
            return errInternal;
        }

        auto *header = static_cast<size_t *>(raw);
        *header = allocation_count;

        page_buffer = reinterpret_cast<ntsplash_page_info_t *>(header + 1);
        if (!page_buffer)
        {
            std::free(raw);
            if (image_buffer)
            {
                ntsplash_renderer_free_image_info(image_buffer);
            }
            for (auto &existing : page_summaries)
            {
                if (existing.colorspaces)
                {
                    ntsplash_renderer_free_page_colorspaces(existing.colorspaces);
                    existing.colorspaces = nullptr;
                    existing.colorspace_count = 0;
                }
            }
            ntsplash_set_error(error_out, "unable to allocate page metadata buffer");
            return errInternal;
        }

        for (size_t i = 0; i < page_summaries.size(); ++i)
        {
            page_buffer[i].page_number = page_summaries[i].page_number;
            page_buffer[i].image_count = page_summaries[i].image_count;
            page_buffer[i].object_count = page_summaries[i].object_count;
            page_buffer[i].is_pdf_a_compatible = page_summaries[i].is_pdf_a_compatible ? 1 : 0;
            page_buffer[i].colorspaces = page_summaries[i].colorspaces;
            page_buffer[i].colorspace_count = page_summaries[i].colorspace_count;

            for (int j = 0; j < 4; ++j)
            {
                page_buffer[i].cropbox[j] = page_summaries[i].cropbox[j];
                page_buffer[i].mediabox[j] = page_summaries[i].mediabox[j];
            }
        }
    }

    *out_images = image_buffer;
    *out_image_len = collected.size();
    *out_pages = page_buffer;
    *out_page_len = page_summaries.size();
    return errNone;
}

void ntsplash_renderer_free_image(ntsplash_image_t *image)
{
    if (!image)
    {
        return;
    }

    if (image->data)
    {
        std::free(image->data);
    }

    image->data = nullptr;
    image->len = 0;
    image->width = 0;
    image->height = 0;
    image->stride = 0;
    image->components = 0;
    image->color_mode = NTSPLASH_COLOR_MODE_RGB8;
    image->bits_per_component = 0;
}

void ntsplash_renderer_free_cstr(char *message) { std::free(message); }

void ntsplash_renderer_free_image_info(ntsplash_image_info_t *images)
{
    if (!images)
    {
        return;
    }

    auto *header = reinterpret_cast<size_t *>(images) - 1;
    const size_t len = *header;

    for (size_t i = 0; i < len; ++i)
    {
        if (images[i].color_space_handle)
        {
            auto *space =
                static_cast<GfxColorSpace *>(const_cast<void *>(images[i].color_space_handle));
            delete space;
            images[i].color_space_handle = nullptr;
        }
    }

    std::free(header);
}

void ntsplash_renderer_free_page_info(ntsplash_page_info_t *pages)
{
    if (!pages)
    {
        return;
    }

    auto *header = reinterpret_cast<size_t *>(pages) - 1;
    const size_t len = *header;

    for (size_t i = 0; i < len; ++i)
    {
        if (pages[i].colorspaces)
        {
            ntsplash_renderer_free_page_colorspaces(
                const_cast<ntsplash_page_colorspace_entry_t *>(pages[i].colorspaces));
            pages[i].colorspaces = nullptr;
            pages[i].colorspace_count = 0;
        }
    }

    std::free(header);
}

void ntsplash_renderer_free_page_colorspaces(ntsplash_page_colorspace_entry_t *entries)
{
    if (!entries)
    {
        return;
    }

    auto *header = reinterpret_cast<size_t *>(entries) - 1;
    const size_t len = *header;

    for (size_t i = 0; i < len; ++i)
    {
        if (entries[i].name)
        {
            std::free(const_cast<char *>(entries[i].name));
            entries[i].name = nullptr;
        }
        if (entries[i].color_space_handle)
        {
            auto *space =
                static_cast<GfxColorSpace *>(const_cast<void *>(entries[i].color_space_handle));
            delete space;
            entries[i].color_space_handle = nullptr;
        }
    }

    std::free(header);
}

void ntsplash_get_version(ntsplash_version_t *out_version)
{
    if (!out_version)
    {
        return;
    }

    // split POPPLER_VERSION into major, minor, patch
    // POPPLER_VERSION is in string format: mm.nn.pp (e.g., 21.03.0)
    uint32_t major = 0;
    uint32_t minor = 0;
    uint32_t patch = 0;

    std::string version_str = POPPLER_VERSION;
    size_t first_dot = version_str.find('.');
    if (first_dot != std::string::npos)
    {
        major = static_cast<uint32_t>(std::stoi(version_str.substr(0, first_dot)));
        size_t second_dot = version_str.find('.', first_dot + 1);
        if (second_dot != std::string::npos)
        {
            minor = static_cast<uint32_t>(
                std::stoi(version_str.substr(first_dot + 1, second_dot - first_dot - 1)));
            patch = static_cast<uint32_t>(std::stoi(version_str.substr(second_dot + 1)));
        }
        else
        {
            minor = static_cast<uint32_t>(std::stoi(version_str.substr(first_dot + 1)));
        }
    }
    else
    {
        major = static_cast<uint32_t>(std::stoi(version_str));
    }

    out_version->major = major;
    out_version->minor = minor;
    out_version->patch = patch;
}

//! Colorspace related functions
ntsplash_image_colorspace_t ntgfxcs_get_color_mode(const void *cs_ptr)
{
    const auto *cs = static_cast<const GfxColorSpace *>(cs_ptr);
    return ntsplash_upconvert_colorspace(cs);
}

bool ntgfxcs_get_indexed_info(const void *cs_ptr, ntcolorspaces_indexed_info_t *out)
{
    const auto *cs = static_cast<const GfxColorSpace *>(cs_ptr);
    if (cs->getMode() != csIndexed)
        return false;

    auto *idxColor =
        const_cast<GfxIndexedColorSpace *>(static_cast<const GfxIndexedColorSpace *>(cs));

    out->hival = idxColor->getIndexHigh();
    out->base = static_cast<void *>(idxColor->getBase());
    return true;
}

bool ntgfxcs_get_separation_info(const void *cs_ptr, ntcolorspaces_separation_info_t *out)
{
    auto *cs = static_cast<const GfxColorSpace *>(cs_ptr);
    if (cs->getMode() != csSeparation)
        return false;

    auto *sep =
        const_cast<GfxSeparationColorSpace *>(static_cast<const GfxSeparationColorSpace *>(cs));

    std::string s = sep->getName()->toStr();

    out->name = strdup(s.c_str());
    out->alternate = static_cast<const void *>(sep->getAlt()); // recursive
    return true;
}

bool ntgfxcs_get_devicen_info(const void *cs_ptr, ntcolorspaces_devicen_info_t *out)
{
    auto *cs = static_cast<const GfxColorSpace *>(cs_ptr);
    if (cs->getMode() != csDeviceN)
        return false;

    // Cast away const only temporarily
    auto *dn = const_cast<GfxDeviceNColorSpace *>(static_cast<const GfxDeviceNColorSpace *>(cs));

    int count = dn->getNComps();
    out->count = (uint32_t)count;

    const char **names = (const char **)malloc(sizeof(char *) * count);

    for (int i = 0; i < count; i++)
    {
        const std::string &s = dn->getColorantName(i);
        names[i] = strdup(s.c_str());
    }
    out->names = names;

    out->alternate = static_cast<const void *>(dn->getAlt());
    return true;
}

bool ntgfxcs_get_labxyz_info(const void *cs_ptr, ntcolorspaces_labxyz_info_t *out)
{
    auto *cs = static_cast<const GfxColorSpace *>(cs_ptr);
    if (cs->getMode() != csLab)
        return false;

    auto *sep = const_cast<GfxLabColorSpace *>(static_cast<const GfxLabColorSpace *>(cs));

    out->whiteX = sep->getWhiteX();
    out->whiteY = sep->getWhiteY();
    out->whiteZ = sep->getWhiteZ();
    out->blackX = sep->getBlackX();
    out->blackY = sep->getBlackY();
    out->blackZ = sep->getBlackZ();
    out->minA = sep->getAMin();
    out->maxA = sep->getAMax();
    out->minB = sep->getBMin();
    out->maxB = sep->getBMax();
    return true;
}

bool ntgfxcs_get_icc_info(const void *cs_ptr, ntcolorspaces_icc_info_t *out)
{
    auto *cs = static_cast<const GfxColorSpace *>(cs_ptr);
    if (cs->getMode() != csICCBased)
        return false;

    auto *sep = const_cast<GfxICCBasedColorSpace *>(static_cast<const GfxICCBasedColorSpace *>(cs));

    out->alternate = static_cast<const void *>(sep->getAlt()); // recursive
    return true;
}

void ntgfxcs_free_string(const char *s) { free((void *)s); }

void ntgfxcs_free_string_array(const char **arr, uint32_t count)
{
    if (!arr)
        return;
    for (uint32_t i = 0; i < count; i++)
    {
        free((void *)arr[i]);
    }
    free((void *)arr);
}
