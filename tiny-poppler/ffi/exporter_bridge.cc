#include "exporter_bridge.h"

#include <climits>
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

void ntsplash_free_image_export(NTImageOutputDev::NTImageOutput &captured)
{
    if (captured.data) {
        gfree(captured.data);
        captured.data = nullptr;
    }
    if (captured.jbig2Globals) {
        gfree(captured.jbig2Globals);
        captured.jbig2Globals = nullptr;
        captured.jbig2GlobalsLen = 0;
    }
}

NTImageOutputDev::NTImageType ntsplash_upconvert_type(nt_image_export_type_t type)
{
    switch (type) {
    case IMAGE_EXPORT_TYPE_STENCIL:
        return NTImageOutputDev::imgStencil;
    case IMAGE_EXPORT_TYPE_MASK:
        return NTImageOutputDev::imgMask;
    case IMAGE_EXPORT_TYPE_SOFT_MASK:
        return NTImageOutputDev::imgSmask;
    case IMAGE_EXPORT_TYPE_IMAGE:
    default:
        return NTImageOutputDev::imgImage;
    }
}

bool ntsplash_validate_export_params(const nt_image_export_params_t *params, char **error_out)
{
    if (!params) {
        ntsplash_set_error(error_out, "missing export parameters");
        return false;
    }

    const bool match_by_ref = params->match_mode == IMAGE_EXPORT_MATCH_BY_REF;
    const bool match_by_occurrence = params->match_mode == IMAGE_EXPORT_MATCH_BY_OCCURRENCE;
    if (!match_by_ref && !match_by_occurrence) {
        ntsplash_set_error(error_out, "unsupported image match mode");
        return false;
    }

    if (match_by_ref) {
        if (params->xref_object <= 0) {
            ntsplash_set_error(error_out, "image match requested without a valid object reference");
            return false;
        }
        if (params->xref_generation < 0) {
            ntsplash_set_error(error_out, "image match requested with a negative generation");
            return false;
        }
    }

    if (match_by_occurrence && params->occurrence_index == UINT32_MAX) {
        ntsplash_set_error(error_out, "occurrence index overflow");
        return false;
    }

    return true;
}

void ntsplash_export_reset_output(nt_image_export_image_t *image)
{
    if (!image) {
        return;
    }
    std::memset(image, 0, sizeof(*image));
}

} // namespace

int ntsplash_exporer_extract_page(ntsplash_renderer_t *renderer,
                           const nt_image_export_params_t *params,
                           nt_image_export_image_t *out_image,
                           bool describe_only,
                           char **error_out)
{
    if (!renderer || !out_image) {
        ntsplash_set_error(error_out, "invalid renderer arguments");
        return errInternal;
    }

    ntsplash_export_reset_output(out_image);

    if (!ntsplash_validate_export_params(params, error_out)) {
        return errInternal;
    }

    if (!renderer->doc || !renderer->doc->isOk()) {
        ntsplash_set_error(error_out, "renderer has no active document");
        return errInternal;
    }

    const int total_pages = renderer->doc->getNumPages();
    if (total_pages <= 0) {
        ntsplash_set_error(error_out, "PDF has no pages to inspect");
        return errBadPageNum;
    }

    const uint32_t zero_based_index = params->page_index;
    const int page_number = static_cast<int>(zero_based_index) + 1;
    if (page_number < 1 || page_number > total_pages) {
        ntsplash_set_error(error_out, "page index out of range");
        return errBadPageNum;
    }

    NTImageOutputDev::NTImageOutput captured {};
    Ref target_ref;
    target_ref.num = params->xref_object;
    target_ref.gen = params->xref_generation;

    const bool match_by_ref = params->match_mode == IMAGE_EXPORT_MATCH_BY_REF;
    if (!match_by_ref) {
        target_ref.num = 0;
        target_ref.gen = 0;
    }

    const bool match_by_occurrence = params->match_mode == IMAGE_EXPORT_MATCH_BY_OCCURRENCE;
    NTImageOutputDev output_dev(&captured,
                              target_ref,
                              match_by_ref,
                              ntsplash_upconvert_type(params->target_type),
                              match_by_occurrence,
                              params->occurrence_index);
    output_dev.setDescribeOnly(describe_only);

    renderer->doc->displayPage(&output_dev, page_number, 72.0, 72.0, 0, true, true, false);

    const int dev_error = output_dev.getErrorCode();
    if (dev_error != errNone) {
        ntsplash_free_image_export(captured);
        ntsplash_set_error(error_out, ntsplash_stringify_error_code(dev_error));
        return dev_error;
    }

    if (!output_dev.hasCaptured()) {
        ntsplash_free_image_export(captured);
        ntsplash_set_error(error_out, "target image was not found on the requested page");
        return errInternal;
    }

    out_image->data = captured.data;
    out_image->len = captured.len;
    out_image->width = captured.width;
    out_image->height = captured.height;
    out_image->stride = captured.stride;
    out_image->components = captured.components;
    out_image->bits_per_component = captured.bits_per_component;
    out_image->width_dpi = captured.wDPI;
    out_image->height_dpi = captured.hDPI;
    out_image->format = static_cast<nt_image_export_format_t>(captured.format);
    out_image->type = static_cast<nt_image_export_type_t>(captured.type);
    out_image->extension = static_cast<nt_image_export_extension_t>(captured.extension);
    out_image->has_jbig2_globals = captured.hasJbig2Globals ? 1 : 0;
    out_image->jbig2_globals = captured.jbig2Globals;
    out_image->jbig2_globals_len = captured.jbig2GlobalsLen;
    captured.jbig2Globals = nullptr;
    captured.jbig2GlobalsLen = 0;
    if (captured.hasCcittParams) {
        out_image->has_ccitt_params = 1;
        out_image->ccitt.encoding = captured.ccittParams.encoding;
        out_image->ccitt.columns = captured.ccittParams.columns;
        out_image->ccitt.rows = captured.ccittParams.rows;
        out_image->ccitt.damaged_rows_before_error = captured.ccittParams.damagedRowsBeforeError;
        out_image->ccitt.end_of_line = captured.ccittParams.endOfLine ? 1 : 0;
        out_image->ccitt.byte_align = captured.ccittParams.byteAlign ? 1 : 0;
        out_image->ccitt.end_of_block = captured.ccittParams.endOfBlock ? 1 : 0;
        out_image->ccitt.black_is_one = captured.ccittParams.blackIs1 ? 1 : 0;
    } else {
        out_image->has_ccitt_params = 0;
    }

    return errNone;
}

void ntsplash_exporter_free(nt_image_export_image_t *image)
{
    if (!image) {
        return;
    }

    if (image->data) {
        gfree(image->data);
    }
    if (image->jbig2_globals) {
        gfree(image->jbig2_globals);
    }

    ntsplash_export_reset_output(image);
}
