#include "exporter_bridge.h"

#include <cstdlib>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>

#include "ErrorCodes.h"
#include "GlobalParams.h"
#include "PDFDoc.h"
#include "Page.h"
#include "goo/gmem.h"
#include "image_exporter.h"
#include "splash_renderer_internal.h"

namespace {

void ensure_global_params()
{
    static std::once_flag once;
    std::call_once(once, [] {
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

ImageOutputDev::ImageType to_image_type(image_export_type_t type)
{
    switch (type) {
    case IMAGE_EXPORT_TYPE_STENCIL:
        return ImageOutputDev::imgStencil;
    case IMAGE_EXPORT_TYPE_MASK:
        return ImageOutputDev::imgMask;
    case IMAGE_EXPORT_TYPE_SOFT_MASK:
        return ImageOutputDev::imgSmask;
    case IMAGE_EXPORT_TYPE_IMAGE:
    default:
        return ImageOutputDev::imgImage;
    }
}

bool validate_params(const image_export_params_t *params, char **error_out)
{
    if (!params) {
        set_error(error_out, "missing export parameters");
        return false;
    }

    const bool match_by_ref = params->match_mode == IMAGE_EXPORT_MATCH_BY_REF;
    const bool match_by_type = params->match_mode == IMAGE_EXPORT_MATCH_BY_TYPE;
    if (!match_by_ref && !match_by_type) {
        set_error(error_out, "unsupported image match mode");
        return false;
    }

    if (match_by_ref) {
        if (params->xref_object <= 0) {
            set_error(error_out, "image match requested without a valid object reference");
            return false;
        }
        if (params->xref_generation < 0) {
            set_error(error_out, "image match requested with a negative generation");
            return false;
        }
    }

    return true;
}

void reset_output(image_export_image_t *image)
{
    if (!image) {
        return;
    }
    std::memset(image, 0, sizeof(*image));
}

} // namespace

int image_exporter_extract(splash_renderer_t *renderer,
                           const image_export_params_t *params,
                           image_export_image_t *out_image,
                           char **error_out)
{
    if (!renderer || !out_image) {
        set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    reset_output(out_image);

    if (!validate_params(params, error_out)) {
        return errInternal;
    }

    ensure_global_params();

    if (!renderer->doc || !renderer->doc->isOk()) {
        set_error(error_out, "renderer has no active document");
        return errInternal;
    }

    const int total_pages = renderer->doc->getNumPages();
    if (total_pages <= 0) {
        set_error(error_out, "PDF has no pages to inspect");
        return errBadPageNum;
    }

    const uint32_t zero_based_index = params->page_index;
    const int page_number = static_cast<int>(zero_based_index) + 1;
    if (page_number < 1 || page_number > total_pages) {
        set_error(error_out, "page index out of range");
        return errBadPageNum;
    }

    ImageOutputDev::ImageOutput captured {};
    Ref target_ref;
    target_ref.num = params->xref_object;
    target_ref.gen = params->xref_generation;

    const bool match_by_ref = params->match_mode == IMAGE_EXPORT_MATCH_BY_REF;
    if (!match_by_ref) {
        target_ref.num = 0;
        target_ref.gen = 0;
    }

    ImageOutputDev output_dev(&captured, target_ref, match_by_ref, to_image_type(params->target_type));

    renderer->doc->displayPage(&output_dev, page_number, 72.0, 72.0, 0, true, true, false);

    const int dev_error = output_dev.getErrorCode();
    if (dev_error != errNone) {
        if (captured.data) {
            gfree(captured.data);
        }
        set_error(error_out, error_code_to_string(dev_error));
        return dev_error;
    }

    if (!output_dev.hasCaptured()) {
        set_error(error_out, "target image was not found on the requested page");
        return errInternal;
    }

    out_image->data = captured.data;
    out_image->len = captured.len;
    out_image->width = captured.width;
    out_image->height = captured.height;
    out_image->stride = captured.stride;
    out_image->components = captured.components;
    out_image->bits_per_component = captured.bits_per_component;
    out_image->format = static_cast<image_export_format_t>(captured.format);
    out_image->type = static_cast<image_export_type_t>(captured.type);
    out_image->extension = static_cast<image_export_extension_t>(captured.extension);

    return errNone;
}

void image_exporter_free(image_export_image_t *image)
{
    if (!image) {
        return;
    }

    if (image->data) {
        gfree(image->data);
    }

    reset_output(image);
}
