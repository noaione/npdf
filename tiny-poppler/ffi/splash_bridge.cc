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

#include "poppler-config.h"
#include "ErrorCodes.h"
#include "GfxState.h"
#include "OutputDev.h"
#include "Stream.h"
#include "GlobalParams.h"
#include "PDFDoc.h"
#include "SplashOutputDev.h"
#include "goo/GooString.h"
#include "splash/SplashBitmap.h"
#include "splash/SplashTypes.h"
#include "splash_renderer_internal.h"

#define VERSION_MAJOR POPP

namespace {
constexpr int kBitmapRowPad = 4;
constexpr bool kReverseVideo = false;
constexpr bool kTopDownBitmap = true;
constexpr SplashThinLineMode kThinLineMode = splashThinLineDefault;

// This flag handles the synchronization state
static std::once_flag init_flag;

std::optional<SplashColorMode> to_splash_color_mode(splash_color_mode_t mode)
{
    switch (mode) {
    case SPLASH_COLOR_MODE_MONO1:
        return splashModeMono1;
    case SPLASH_COLOR_MODE_MONO8:
        return splashModeMono8;
    case SPLASH_COLOR_MODE_RGB8:
        return splashModeRGB8;
    case SPLASH_COLOR_MODE_BGR8:
        return splashModeBGR8;
    case SPLASH_COLOR_MODE_XBGR8:
        return splashModeXBGR8;
    case SPLASH_COLOR_MODE_CMYK8:
        return splashModeCMYK8;
    case SPLASH_COLOR_MODE_DEVICEN8:
        return splashModeDeviceN8;
    default:
        return std::nullopt;
    }
}

void ensure_global_params()
{
    std::call_once(init_flag, []() {
        globalParams = std::make_unique<GlobalParams>();
        globalParams->setErrQuiet(true);
    });
}

void set_error(char **error_out, const std::string &message)
{
    if (!error_out) {
        return;
    }
    *error_out = nullptr;
    const size_t len = message.size();
    char *buffer = static_cast<char *>(std::malloc(len + 1));
    if (!buffer) {
        return;
    }
    std::memcpy(buffer, message.c_str(), len);
    buffer[len] = '\0';
    *error_out = buffer;
}

std::string error_code_to_string(int error_code)
{
    switch (error_code) {
    case errNone:
        return "ok";
    case errOpenFile:
        return "failed to open PDF";
    case errBadCatalog:
        return "invalid PDF catalog";
    case errDamaged:
        return "PDF is damaged and could not be repaired";
    case errEncrypted:
        return "PDF is encrypted and no password was provided";
    case errHighlightFile:
        return "invalid highlight file";
    case errBadPrinter:
        return "invalid printer configuration";
    case errPrinting:
        return "error while printing";
    case errPermission:
        return "operation not permitted by PDF";
    case errBadPageNum:
        return "invalid page number";
    case errFileIO:
        return "file I/O failure";
    case errFileChangedSinceOpen:
        return "PDF changed since open";
    default:
        return "unknown poppler error";
    }
}

splash_image_colorspace_t to_image_colorspace(const GfxColorSpace *color_space)
{
    if (!color_space) {
        return SPLASH_IMAGE_COLORSPACE_UNKNOWN;
    }

    switch (color_space->getMode()) {
    case GfxColorSpaceMode::csDeviceGray:
    case GfxColorSpaceMode::csCalGray:
        return SPLASH_IMAGE_COLORSPACE_DEVICE_GRAY;
    case GfxColorSpaceMode::csDeviceRGB:
    case GfxColorSpaceMode::csCalRGB:
        return SPLASH_IMAGE_COLORSPACE_DEVICE_RGB;
    case GfxColorSpaceMode::csDeviceCMYK:
        return SPLASH_IMAGE_COLORSPACE_DEVICE_CMYK;
    case GfxColorSpaceMode::csLab:
        return SPLASH_IMAGE_COLORSPACE_LAB;
    case GfxColorSpaceMode::csICCBased:
        return SPLASH_IMAGE_COLORSPACE_ICC;
    case GfxColorSpaceMode::csIndexed:
        return SPLASH_IMAGE_COLORSPACE_INDEXED;
    case GfxColorSpaceMode::csPattern:
        return SPLASH_IMAGE_COLORSPACE_PATTERN;
    case GfxColorSpaceMode::csSeparation:
        return SPLASH_IMAGE_COLORSPACE_SEPARATION;
    case GfxColorSpaceMode::csDeviceN:
        return SPLASH_IMAGE_COLORSPACE_DEVICEN;
    default:
        return SPLASH_IMAGE_COLORSPACE_UNKNOWN;
    }
}

bool copy_bitmap_to_image(SplashBitmap *bitmap, SplashColorMode mode, splash_image_t *out_image, char **error_out)
{
    if (!bitmap || !out_image) {
        set_error(error_out, "internal splash renderer error");
        return false;
    }

    const int width = bitmap->getWidth();
    const int height = bitmap->getHeight();
    const int row_size = bitmap->getRowSize();

    if (width <= 0 || height <= 0 || row_size <= 0) {
        set_error(error_out, "received empty bitmap from renderer");
        return false;
    }

    const size_t stride = static_cast<size_t>(row_size);
    const size_t total_size = stride * static_cast<size_t>(height);

    auto *buffer = static_cast<uint8_t *>(std::malloc(total_size));
    if (!buffer) {
        set_error(error_out, "unable to allocate buffer for rendered page");
        return false;
    }

    std::memcpy(buffer, bitmap->getDataPtr(), total_size);

    const SplashColorMode bitmap_mode = bitmap->getMode();
    if (bitmap_mode != mode) {
        std::free(buffer);
        set_error(error_out, "renderer returned bitmap with unexpected color mode");
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
    out_image->color_mode = static_cast<splash_color_mode_t>(bitmap_mode);
    out_image->bits_per_component = bits_per_component;

    return true;
}

struct CollectedImage {
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t components = 0;
    uint32_t bits_per_component = 0;
    int32_t xref_object = -1;
    int32_t xref_generation = 0;
    uint32_t page_number = 0;
    double_t dpi_x = 0;
    double_t dpi_y = 0;
    splash_image_type_t image_type = SPLASH_IMAGE_TYPE_UNKNOWN;
    splash_image_colorspace_t colorspace = SPLASH_IMAGE_COLORSPACE_UNKNOWN;
    double ctm[6] = {1.0, 0.0, 0.0, 1.0, 0.0, 0.0}; // Default is identity matrix
    const void *color_space_handle = nullptr;
};

struct CollectedPage {
    uint32_t page_number = 0;
    uint32_t image_count = 0;
    uint64_t object_count = 0;

    // If all 0's, then not set
    double cropbox[4] = {0.0, 0.0, 0.0, 0.0};
    double mediabox[4] = {0.0, 0.0, 0.0, 0.0};
};

const void *copy_color_space(const GfxColorSpace *space)
{
    if (!space) {
        return nullptr;
    }

    std::unique_ptr<GfxColorSpace> copy = space->copy();
    return static_cast<const void *>(copy.release());
}

class ImageCollector final : public OutputDev
{
public:
    explicit ImageCollector(std::vector<CollectedImage> *images)
        : images_(images)
    {
    }

    bool upsideDown() override { return false; }
    bool useDrawChar() override { return false; }
    bool interpretType3Chars() override { return false; }

    void reset_for_page(uint32_t page_number) {
        current_page_ = page_number;
        total_objects_ = 0; // reset object count for new page
    }

    uint64_t get_total_objects() const { return total_objects_; }

    void drawImage(GfxState *state, Object *ref, Stream *str, int width, int height, GfxImageColorMap *color_map, bool interpolate, const int *maskColors, bool inlineImg) override
    {
        (void)state;
        (void)str;
        (void)maskColors;
        (void)inlineImg;
        (void)interpolate;
        total_objects_++;
        add_image(width, height, color_map, ref, state, SPLASH_IMAGE_TYPE_IMAGE);
    }

    void drawImageMask(GfxState *state, Object *ref, Stream *str, int width, int height, bool invert, bool interpolate, bool inlineImg) override
    {
        (void)state;
        (void)str;
        (void)invert;
        (void)interpolate;
        (void)inlineImg;
        total_objects_++;
        add_mask(width, height, ref, state);
    }

    void drawMaskedImage(GfxState *state,
                         Object *ref,
                         Stream *str,
                         int width,
                         int height,
                         GfxImageColorMap *color_map,
                         bool interpolate,
                         Stream *maskStr,
                         int maskWidth,
                         int maskHeight,
                         bool maskInvert,
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
        add_image(width, height, color_map, ref, state, SPLASH_IMAGE_TYPE_IMAGE);
        add_image(maskWidth, maskHeight, nullptr, ref, state, SPLASH_IMAGE_TYPE_MASK);
    }

    void drawSoftMaskedImage(GfxState *state,
                             Object *ref,
                             Stream *str,
                             int width,
                             int height,
                             GfxImageColorMap *color_map,
                             bool interpolate,
                             Stream *maskStr,
                             int maskWidth,
                             int maskHeight,
                             GfxImageColorMap *maskColorMap,
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
        add_image(width, height, color_map, ref, state, SPLASH_IMAGE_TYPE_IMAGE);
        add_image(maskWidth, maskHeight, maskColorMap, ref, state, SPLASH_IMAGE_TYPE_SOFT_MASK);
    }

    void drawString(GfxState *state, const GooString *s) override
    {
        (void)state;
        (void)s;
        total_objects_++;
    }

    void drawForm(Ref id) override
    {
        (void)id;
        total_objects_++;
    }

    void stroke(GfxState *state) override
    {
        (void)state;
        total_objects_++;
    }

    void fill(GfxState *state) override
    {
        (void)state;
        total_objects_++;
    }

    void eoFill(GfxState *state) override
    {
        (void)state;
        total_objects_++;
    }

    void clip(GfxState *state) override
    {
        (void)state;

        // Check clip
        bool is_clip_empty = state->isPath();
        if (!is_clip_empty) {
            total_objects_++;
        }
    }

    void eoClip(GfxState *state) override
    {
        (void)state;
        bool is_clip_empty = state->isPath();
        if (!is_clip_empty) {
            total_objects_++;
        }
    }

    void psXObject(Stream *psStream, Stream *level1Stream) override
    {
        (void)psStream;
        (void)level1Stream;
        total_objects_++;
    }

private:
    void add_image(
        int width,
        int height,
        GfxImageColorMap *color_map,
        Object *ref,
        GfxState *state,
        splash_image_type_t image_type
    )
    {
        if (!images_) {
            return;
        }

        CollectedImage info;
        info.page_number = current_page_;
        if (width > 0) {
            info.width = static_cast<uint32_t>(width);
        }
        if (height > 0) {
            info.height = static_cast<uint32_t>(height);
        }
        info.image_type = image_type;

        if (color_map) {
            info.components = static_cast<uint32_t>(color_map->getNumPixelComps());
            info.bits_per_component = static_cast<uint32_t>(color_map->getBits());
            const GfxColorSpace *space = color_map->getColorSpace();
            info.colorspace = to_image_colorspace(space);
            info.color_space_handle = copy_color_space(space);
        } else {
            info.components = 1;
            info.bits_per_component = 1;
            info.colorspace = SPLASH_IMAGE_COLORSPACE_DEVICE_GRAY;
            info.color_space_handle = nullptr;
        }

        if (ref && ref->isRef()) {
            const auto reference = ref->getRef();
            info.xref_object = static_cast<int32_t>(reference.num);
            info.xref_generation = static_cast<int32_t>(reference.gen);
        }

        if (state) {
            std::pair<double, double> dpi = calculate_image_dpi(state->getCTM(), width, height);
            info.dpi_x = static_cast<double_t>(dpi.first);
            info.dpi_y = static_cast<double_t>(dpi.second);

            const double *ctm = state->getCTM();
            if (ctm) {
                std::memcpy(info.ctm, ctm, 6 * sizeof(double));
            }
        }

        images_->push_back(info);
    }

    void add_mask(int width, int height, Object *ref, GfxState *state)
    {
        if (!images_) {
            return;
        }

        CollectedImage info;
        info.page_number = current_page_;
        info.image_type = SPLASH_IMAGE_TYPE_STENCIL;
        if (width > 0) {
            info.width = static_cast<uint32_t>(width);
        }
        if (height > 0) {
            info.height = static_cast<uint32_t>(height);
        }
        info.components = 1;
        info.bits_per_component = 1;
        info.colorspace = SPLASH_IMAGE_COLORSPACE_DEVICE_GRAY;
        info.color_space_handle = nullptr;
        if (ref && ref->isRef()) {
            const auto reference = ref->getRef();
            info.xref_object = static_cast<int32_t>(reference.num);
            info.xref_generation = static_cast<int32_t>(reference.gen);
        }
        if (state) {
            std::pair<double, double> dpi = calculate_image_dpi(state->getCTM(), width, height);
            info.dpi_x = static_cast<double_t>(dpi.first);
            info.dpi_y = static_cast<double_t>(dpi.second);
            const double *ctm = state->getCTM();
            if (ctm) {
                std::memcpy(info.ctm, ctm, 6 * sizeof(double));
            }
        }
        images_->push_back(info);
    }

    std::pair<double, double> calculate_image_dpi(const double *ctm, int width, int height)
    {
        if (!ctm) {
            return {0.0, 0.0};
        }

        // Calculate the scaling factors from the CTM
        double width2 = sqrt(ctm[0] * ctm[0] + ctm[1] * ctm[1]);
        double height2 = sqrt(ctm[2] * ctm[2] + ctm[3] * ctm[3]);
        
        double xppi = fabs(width * 72.0 / width2);
        double yppi = fabs(height * 72.0 / height2);

        return {xppi, yppi};
    }

    std::vector<CollectedImage> *images_ = nullptr;
    uint32_t current_page_ = 0;
    uint64_t total_objects_ = 0;
};

} // namespace

int splash_renderer_create(const char *path,
                           const char *owner_password,
                           const char *user_password,
                           splash_renderer_t **out_renderer,
                           char **error_out)
{
    if (!path || !out_renderer) {
        set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    ensure_global_params();

    auto goo_path = std::make_unique<GooString>(path);
    std::optional<GooString> owner_pw;
    if (owner_password != nullptr) {
        owner_pw.emplace(owner_password);
    }

    std::optional<GooString> user_pw;
    if (user_password != nullptr) {
        user_pw.emplace(user_password);
    }

    std::unique_ptr<PDFDoc> doc =
        std::make_unique<PDFDoc>(std::move(goo_path), owner_pw, user_pw);

    if (!doc->isOk()) {
        const int error_code = doc->getErrorCode();
        set_error(error_out, error_code_to_string(error_code));
        return error_code == 0 ? errInternal : error_code;
    }

    auto renderer = std::make_unique<splash_renderer>();
    renderer->doc = std::move(doc);

    *out_renderer = renderer.release();
    return errNone;
}

void splash_renderer_destroy(splash_renderer_t *renderer)
{
    if (!renderer) {
        return;
    }
    delete renderer;
}

int splash_renderer_page_count(splash_renderer_t *renderer, uint32_t *out_count, char **error_out)
{
    if (!renderer || !out_count) {
        set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    const int count = renderer->doc->getNumPages();
    if (count < 0) {
        set_error(error_out, "failed to query page count");
        return errInternal;
    }

    *out_count = static_cast<uint32_t>(count);
    return errNone;
}

int splash_renderer_render_page(splash_renderer_t *renderer,
                                uint32_t page_index,
                                double dpi,
                                splash_color_mode_t color_mode,
                                splash_crop_mode_t crop_mode,
                                splash_image_t *out_image,
                                char **error_out)
{
    if (!renderer || !out_image) {
        set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    const int page_number = static_cast<int>(page_index) + 1;
    const int total_pages = renderer->doc->getNumPages();
    if (page_number < 1 || page_number > total_pages) {
        set_error(error_out, "page index out of range");
        return errBadPageNum;
    }

    auto maybe_mode = to_splash_color_mode(color_mode);
    if (!maybe_mode) {
        set_error(error_out, "unsupported Splash color mode requested");
        return errInternal;
    }

    Page *page = renderer->doc->getPage(page_number); // Preload page to set up crop boxes, etc.

    const double clamped_dpi = dpi > 0.0 ? dpi : 72.0;
    bool use_media_box = crop_mode == SPLASH_CROP_MODE_MEDIA_BOX;

    const SplashColorMode requested_mode = *maybe_mode;
    const bool enable_overprint =
        color_mode == SPLASH_COLOR_MODE_CMYK8 || color_mode == SPLASH_COLOR_MODE_DEVICEN8;

    SplashColor paper_color;
    if (enable_overprint) {
        splashClearColor(paper_color);
    } else {
        paper_color[0] = 255;
        paper_color[1] = 255;
        paper_color[2] = 255;
        // paper_color[3] = 255;
    }

    SplashOutputDev output_dev(requested_mode,
                               kBitmapRowPad,
                               kReverseVideo,
                               paper_color,
                               kTopDownBitmap,
                               kThinLineMode,
                               enable_overprint);
    output_dev.setVectorAntialias(true);
    output_dev.setFontAntialias(true);
    output_dev.setEnableFreeType(true);
    output_dev.setFreeTypeHinting(true, true);
    output_dev.startDoc(renderer->doc.get());

    page->display(
        &output_dev,
        clamped_dpi,
        clamped_dpi,
        0,
        use_media_box,
        false,
        false
    );

    std::unique_ptr<SplashBitmap> bitmap(output_dev.takeBitmap());
    if (!bitmap) {
        set_error(error_out, "renderer produced no bitmap");
        return errInternal;
    }

    if (!copy_bitmap_to_image(bitmap.get(), requested_mode, out_image, error_out)) {
        return errInternal;
    }

    return errNone;
}

int splash_renderer_collect_images(splash_renderer_t *renderer,
                                   splash_image_info_t **out_images,
                                   size_t *out_image_len,
                                   splash_page_info_t **out_pages,
                                   size_t *out_page_len,
                                   uint32_t page_start,
                                   uint32_t page_end,
                                   char **error_out)
{
    if (!renderer || !out_images || !out_image_len || !out_pages || !out_page_len) {
        set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    *out_images = nullptr;
    *out_image_len = 0;
    *out_pages = nullptr;
    *out_page_len = 0;

    const int total_pages = renderer->doc->getNumPages();
    if (total_pages <= 0) {
        return errNone;
    }

    uint32_t start_page = page_start > 0 ? page_start : 1;
    uint32_t end_page = page_end > 0 ? page_end : static_cast<uint32_t>(total_pages);
    if (start_page < 1 || start_page > static_cast<uint32_t>(total_pages)) {
        set_error(error_out, "start page out of range");
        return errBadPageNum;
    }
    if (end_page < start_page || end_page > static_cast<uint32_t>(total_pages)) {
        set_error(error_out, "end page out of range");
        return errBadPageNum;
    }

    std::vector<CollectedImage> collected;
    collected.reserve(static_cast<size_t>(total_pages));

    const uint32_t page_span = end_page - start_page + 1;
    std::vector<CollectedPage> page_summaries;
    page_summaries.reserve(static_cast<size_t>(page_span));

    ImageCollector collector(&collected);

    for (uint32_t page_number = start_page; page_number <= end_page; ++page_number) {
        collector.reset_for_page(page_number);
        const size_t before = collected.size();
        Page *page = renderer->doc->getPage(static_cast<int>(page_number));
        page->display(&collector, 72.0, 72.0, 0, true, true, false);
        const size_t after = collected.size();

        const PDFRectangle *cropbox = page->getCropBox();
        const PDFRectangle *mediabox = page->getMediaBox();

        CollectedPage summary;
        summary.page_number = page_number;
        summary.image_count = static_cast<uint32_t>(after - before);
        summary.object_count = collector.get_total_objects();

        if (cropbox) {
            summary.cropbox[0] = cropbox->x1;
            summary.cropbox[1] = cropbox->y1;
            summary.cropbox[2] = cropbox->x2;
            summary.cropbox[3] = cropbox->y2;
        }
        if (mediabox) {
            summary.mediabox[0] = mediabox->x1;
            summary.mediabox[1] = mediabox->y1;
            summary.mediabox[2] = mediabox->x2;
            summary.mediabox[3] = mediabox->y2;
        }
        page_summaries.push_back(summary);
    }

    splash_image_info_t *image_buffer = nullptr;
    if (!collected.empty()) {
        const size_t allocation_count = collected.size();
        const size_t header_size = sizeof(size_t);
        const size_t payload_size = allocation_count * sizeof(splash_image_info_t);
        void *raw = std::malloc(header_size + payload_size);
        if (!raw) {
            set_error(error_out, "unable to allocate image metadata buffer");
            return errInternal;
        }

        auto *header = static_cast<size_t *>(raw);
        *header = allocation_count;

        image_buffer = reinterpret_cast<splash_image_info_t *>(header + 1);
        if (!image_buffer) {
            std::free(raw);
            set_error(error_out, "unable to allocate image metadata buffer");
            return errInternal;
        }

        for (size_t i = 0; i < collected.size(); ++i) {
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
            for (int j = 0; j < 6; ++j) {
                image_buffer[i].ctm[j] = collected[i].ctm[j];
            }
        }
    }

    splash_page_info_t *page_buffer = nullptr;
    if (!page_summaries.empty()) {
        const size_t allocation_count = page_summaries.size();
        const size_t header_size = sizeof(size_t);
        const size_t payload_size = allocation_count * sizeof(splash_page_info_t);
        void *raw = std::malloc(header_size + payload_size);
        if (!raw) {
            if (image_buffer) {
                splash_renderer_free_image_info(image_buffer);
            }
            set_error(error_out, "unable to allocate page metadata buffer");
            return errInternal;
        }

        auto *header = static_cast<size_t *>(raw);
        *header = allocation_count;

        page_buffer = reinterpret_cast<splash_page_info_t *>(header + 1);
        if (!page_buffer) {
            std::free(raw);
            if (image_buffer) {
                splash_renderer_free_image_info(image_buffer);
            }
            set_error(error_out, "unable to allocate page metadata buffer");
            return errInternal;
        }

        for (size_t i = 0; i < page_summaries.size(); ++i) {
            page_buffer[i].page_number = page_summaries[i].page_number;
            page_buffer[i].image_count = page_summaries[i].image_count;
            page_buffer[i].object_count = page_summaries[i].object_count;

            for (int j = 0; j < 4; ++j) {
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

void splash_renderer_free_image(splash_image_t *image)
{
    if (!image) {
        return;
    }

    if (image->data) {
        std::free(image->data);
    }

    image->data = nullptr;
    image->len = 0;
    image->width = 0;
    image->height = 0;
    image->stride = 0;
    image->components = 0;
    image->color_mode = SPLASH_COLOR_MODE_RGB8;
    image->bits_per_component = 0;
}

void splash_renderer_free_cstr(char *message)
{
    std::free(message);
}

void splash_renderer_free_image_info(splash_image_info_t *images)
{
    if (!images) {
        return;
    }

    auto *header = reinterpret_cast<size_t *>(images) - 1;
    const size_t len = *header;

    for (size_t i = 0; i < len; ++i) {
        if (images[i].color_space_handle) {
            auto *space = static_cast<GfxColorSpace *>(const_cast<void *>(images[i].color_space_handle));
            delete space;
            images[i].color_space_handle = nullptr;
        }
    }

    std::free(header);
}

void splash_renderer_free_page_info(splash_page_info_t *pages)
{
    if (!pages) {
        return;
    }

    auto *header = reinterpret_cast<size_t *>(pages) - 1;
    std::free(header);
}

void splash_get_version(splash_version_t *out_version) {
    if (!out_version) {
        return;
    }

    // split POPPLER_VERSION into major, minor, patch
    // POPPLER_VERSION is in string format: mm.nn.pp (e.g., 21.03.0)
    uint32_t major = 0;
    uint32_t minor = 0;
    uint32_t patch = 0;

    std::string version_str = POPPLER_VERSION;
    size_t first_dot = version_str.find('.');
    if (first_dot != std::string::npos) {
        major = static_cast<uint32_t>(std::stoi(version_str.substr(0, first_dot)));
        size_t second_dot = version_str.find('.', first_dot + 1);
        if (second_dot != std::string::npos) {
            minor = static_cast<uint32_t>(std::stoi(version_str.substr(first_dot + 1, second_dot - first_dot - 1)));
            patch = static_cast<uint32_t>(std::stoi(version_str.substr(second_dot + 1)));
        } else {
            minor = static_cast<uint32_t>(std::stoi(version_str.substr(first_dot + 1)));
        }
    } else {
        major = static_cast<uint32_t>(std::stoi(version_str));
    }

    out_version->major = major;
    out_version->minor = minor;
    out_version->patch = patch;
}

//! Colorspace related functions
splash_image_colorspace_t gfxcs_get_color_mode(const void *cs_ptr) {
    const auto *cs = static_cast<const GfxColorSpace*>(cs_ptr);
    return to_image_colorspace(cs);
}

bool gfxcs_get_indexed_info(const void *cs_ptr, colorspaces_indexed_info_t *out) {
    const auto *cs = static_cast<const GfxColorSpace*>(cs_ptr);
    if (cs->getMode() != csIndexed) return false;

    auto *idxColor = const_cast<GfxIndexedColorSpace*>(
        static_cast<const GfxIndexedColorSpace*>(cs)
    );

    out->hival = idxColor->getIndexHigh();
    out->base = static_cast<void*>(idxColor->getBase());
    return true;
}

bool gfxcs_get_separation_info(const void *cs_ptr, colorspaces_separation_info_t *out) {
    auto *cs = static_cast<const GfxColorSpace*>(cs_ptr);
    if (cs->getMode() != csSeparation) return false;

    auto *sep = const_cast<GfxSeparationColorSpace*>(
        static_cast<const GfxSeparationColorSpace*>(cs)
    );

    std::string s = sep->getName()->toStr();

    out->name = strdup(s.c_str());
    out->alternate = static_cast<const void*>(sep->getAlt());  // recursive
    return true;
}

bool gfxcs_get_devicen_info(const void *cs_ptr, colorspaces_devicen_info_t *out) {
    auto *cs = static_cast<const GfxColorSpace*>(cs_ptr);
    if (cs->getMode() != csDeviceN) return false;

    // Cast away const only temporarily
    auto *dn = const_cast<GfxDeviceNColorSpace*>(
        static_cast<const GfxDeviceNColorSpace*>(cs)
    );

    int count = dn->getNComps();
    out->count = (uint32_t)count;

    const char **names = (const char**) malloc(sizeof(char*) * count);

    for (int i = 0; i < count; i++) {
        const std::string &s = dn->getColorantName(i);
        names[i] = strdup(s.c_str());
    }
    out->names = names;

    out->alternate = static_cast<const void*>(dn->getAlt());
    return true;
}

bool gfxcs_get_labxyz_info(const void *cs_ptr, colorspaces_labxyz_info_t *out) {
    auto *cs = static_cast<const GfxColorSpace*>(cs_ptr);
    if (cs->getMode() != csLab) return false;

    auto *sep = const_cast<GfxLabColorSpace*>(
        static_cast<const GfxLabColorSpace*>(cs)
    );

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

bool gfxcs_get_icc_info(const void *cs_ptr, colorspaces_icc_info_t *out) {
    auto *cs = static_cast<const GfxColorSpace*>(cs_ptr);
    if (cs->getMode() != csICCBased) return false;

    auto *sep = const_cast<GfxICCBasedColorSpace*>(
        static_cast<const GfxICCBasedColorSpace*>(cs)
    );

    out->alternate = static_cast<const void*>(sep->getAlt());  // recursive
    return true;
}

void gfxcs_free_string(const char *s) {
    free((void*)s);
}

void gfxcs_free_string_array(const char **arr, uint32_t count) {
    if (!arr) return;
    for (uint32_t i = 0; i < count; i++) {
        free((void*)arr[i]);
    }
    free((void*)arr);
}
