#include "bridge.h"

#include <csetjmp>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>

#include <jpeglib.h>
#include <lib/jpegli/encode.h>
#include <version.h>

#define SJPEGLI_BAD_COLORSPACE 10 // Based on libjpeg error codes
#define SJPEGLI_BAD_INPUT 90      // Custom error code for bad input
#define SJPEGLI_BAD_DPI 91        // Custom error code for bad DPI values

#define SJPEGLI_ERROR 0
#define SJPEGLI_SUCCESS 1

namespace {
constexpr int kDisableProgressive = 0;
constexpr int kDefaultProgressiveLevel = 2;

struct JpegliErrorManager {
    struct jpeg_error_mgr pub;
    jmp_buf jump_buffer;
    char last_error_msg[JPEGLI_ERR_MSG_LEN];
};

void sjpegli_error_exit(j_common_ptr cinfo)
{
    auto *myerr = reinterpret_cast<JpegliErrorManager *>(cinfo->err);
    (*cinfo->err->format_message)(cinfo, myerr->last_error_msg);
    longjmp(myerr->jump_buffer, 1); // jump to exit point
}

J_COLOR_SPACE sjpegli_convert_colorspace(simple_jpegli_colorspace_t colorspace)
{
    switch (colorspace)
    {
    case SJ_GRAYSCALE:
        return JCS_GRAYSCALE;
    case SJ_RGB:
        return JCS_RGB;
    case SJ_YCbCr:
        return JCS_YCbCr;
    case SJ_CMYK:
        return JCS_CMYK;
    case SJ_YCCK:
        return JCS_YCCK;
    case SJ_EXT_RGB:
        return JCS_EXT_RGB;
    case SJ_EXT_RGBX:
        return JCS_EXT_RGBX;
    case SJ_EXT_BGR:
        return JCS_EXT_BGR;
    case SJ_EXT_BGRX:
        return JCS_EXT_BGRX;
    case SJ_EXT_XBGR:
        return JCS_EXT_XBGR;
    case SJ_EXT_XRGB:
        return JCS_EXT_XRGB;
    case SJ_EXT_RGBA:
        return JCS_EXT_RGBA;
    case SJ_EXT_BGRA:
        return JCS_EXT_BGRA;
    case SJ_EXT_ABGR:
        return JCS_EXT_ABGR;
    case SJ_EXT_ARGB:
        return JCS_EXT_ARGB;
    case SJ_RGB565:
        return JCS_RGB565;
    default:
        return JCS_UNKNOWN;
    }
}

int sjpegli_get_input_comps(J_COLOR_SPACE colorspace, int width)
{
    switch (colorspace)
    {
    case JCS_GRAYSCALE:
        return 1;
    case JCS_RGB:
    case JCS_YCbCr:
        return 3;
    case JCS_CMYK:
    case JCS_YCCK:
        return 4;
    case JCS_EXT_RGB:
    case JCS_EXT_BGR:
        return 3;
    case JCS_EXT_RGBX:
    case JCS_EXT_BGRX:
    case JCS_EXT_XBGR:
    case JCS_EXT_XRGB:
        return 4;
    case JCS_EXT_RGBA:
    case JCS_EXT_BGRA:
    case JCS_EXT_ABGR:
    case JCS_EXT_ARGB:
        return 4;
    case JCS_RGB565:
        return 2;
    default:
        return 0; // Unknown colorspace
    }
}

void sjpegli_auto_subsampling_factors(j_compress_ptr cinfo, J_COLOR_SPACE colorspace, int quality)
{
    if (cinfo == nullptr || cinfo->comp_info == nullptr)
    {
        return;
    }

    int clamped_quality = quality;
    if (clamped_quality < 1)
    {
        clamped_quality = 1;
    }
    else if (clamped_quality > 100)
    {
        clamped_quality = 100;
    }

    for (int comp = 0; comp < cinfo->num_components; ++comp)
    {
        cinfo->comp_info[comp].h_samp_factor = 1;
        cinfo->comp_info[comp].v_samp_factor = 1;
    }

    auto recompute_max_sampling = [&]() {
        int max_h = 1;
        int max_v = 1;
        for (int comp = 0; comp < cinfo->num_components; ++comp)
        {
            if (cinfo->comp_info[comp].h_samp_factor > max_h)
            {
                max_h = cinfo->comp_info[comp].h_samp_factor;
            }
            if (cinfo->comp_info[comp].v_samp_factor > max_v)
            {
                max_v = cinfo->comp_info[comp].v_samp_factor;
            }
        }

        cinfo->max_h_samp_factor = max_h;
        cinfo->max_v_samp_factor = max_v;
    };

    auto set_luma_sampling = [&](int h_factor, int v_factor) {
        if (cinfo->num_components == 0)
        {
            return;
        }
        cinfo->comp_info[0].h_samp_factor = h_factor;
        cinfo->comp_info[0].v_samp_factor = v_factor;
        recompute_max_sampling();
    };

    // YCbCr and YCCK benefit the most from chroma subsampling.
    if (colorspace == JCS_YCbCr)
    {
        if (clamped_quality >= 90)
        {
            // High quality keeps full 4:4:4 sampling.
            recompute_max_sampling();
            return;
        }
        if (clamped_quality >= 70)
        {
            // Middle quality prefers 4:2:2 to reduce size without destroying detail.
            set_luma_sampling(2, 1);
            return;
        }
        // Low quality falls back to 4:2:0 for maximum size reduction.
        set_luma_sampling(2, 2);
        return;
    }

    if (colorspace == JCS_YCCK)
    {
        if (clamped_quality >= 85)
        {
            recompute_max_sampling();
            return;
        }
        if (clamped_quality >= 65)
        {
            set_luma_sampling(2, 1);
            return;
        }
        set_luma_sampling(2, 2);
        return;
    }

    // Everything else (RGB, CMYK, extended RGB/alpha inputs, grayscale, RGB565, etc.)
    // keeps full resolution to avoid channel mismatch artifacts.
    recompute_max_sampling();
}

void sjpegli_set_subsampling_factors(j_compress_ptr cinfo, J_COLOR_SPACE colorspace,
                                     simple_jpegli_subsampling_t subsampling, int quality)
{
    switch (subsampling)
    {
    case SJ_SUBSAMP_S420:
        cinfo->comp_info[0].h_samp_factor = 2;
        cinfo->comp_info[0].v_samp_factor = 2;
        return;
    case SJ_SUBSAMP_S422:
        cinfo->comp_info[0].h_samp_factor = 2;
        cinfo->comp_info[0].v_samp_factor = 1;
        return;
    case SJ_SUBSAMP_S440:
        cinfo->comp_info[0].h_samp_factor = 1;
        cinfo->comp_info[0].v_samp_factor = 2;
        return;
    case SJ_SUBSAMP_S444:
        cinfo->comp_info[0].h_samp_factor = 1;
        cinfo->comp_info[0].v_samp_factor = 1;
        return;
    case SJ_SUBSAMP_NONE:
        // Keep original sampling factors
        return;
    case SJ_SUBSAMP_AUTO:
    default:
        sjpegli_auto_subsampling_factors(cinfo, colorspace, quality);
        return;
    }
}

/**
 * Safe-er string copy without strcopy/strncpy pitfalls
 *
 * Similar to curlx_strcopy from curl
 * (c) cURL contributors, licensed under cURL license.
 */
void sjpegli_strcopy(char *dest, const char *src, size_t dest_size)
{
    if (dest_size == 0)
    {
        return;
    }

    size_t src_len = std::strlen(src);
    if (src_len < dest_size)
    {
        memcpy(dest, src, src_len);
        dest[src_len] = '\0';
    }
    else if (dest_size)
    {
        // fail-safe, don't copy anything if it won't fit
        dest[0] = '\0';
    }
}
} // namespace

simple_jpegli_enc_result sjpegli_encode_pixels(const unsigned char *pixels,
                                               const simple_jpegli_enc_config *config)
{
    // safe default to avoid UB
    simple_jpegli_enc_result result;
    result.data = nullptr;
    result.size = 0;
    result.success = 0;
    result.error_code = 0;
    std::memset(result.error_message, 0, JPEGLI_ERR_MSG_LEN);

    if (pixels == nullptr || config == nullptr)
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_INPUT;
        sjpegli_strcopy(result.error_message, "Invalid input, got nullptr", JPEGLI_ERR_MSG_LEN);
        return result;
    }

    int width = config->width;
    int height = config->height;
    int quality = config->quality;
    unsigned int x_dpi = config->x_dpi;
    unsigned int y_dpi = config->y_dpi;
    simple_jpegli_colorspace_t colorspace = config->colorspace;
    simple_jpegli_subsampling_t subsampling = config->subsampling;

    if (quality < 1 || quality > 100)
    {
        quality = 90; // reset to default quality
    }

    // convert to libjpeg colorspace
    J_COLOR_SPACE colorspace_t = sjpegli_convert_colorspace(colorspace);
    if (colorspace_t == JCS_UNKNOWN)
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_COLORSPACE;
        sjpegli_strcopy(result.error_message, "Unsupported colorspace", JPEGLI_ERR_MSG_LEN);
        return result;
    }

    int input_comps = sjpegli_get_input_comps(colorspace_t, width);
    if (input_comps == 0)
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_COLORSPACE;
        sjpegli_strcopy(result.error_message, "Unsupported colorspace", JPEGLI_ERR_MSG_LEN);
        return result;
    }

    // check if x_dpi and y_dpi is within uint16
    if (x_dpi > UINT16_MAX || y_dpi > UINT16_MAX)
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_DPI;
        sjpegli_strcopy(result.error_message, "DPI values cannot exceed 65535", JPEGLI_ERR_MSG_LEN);
        return result;
    }
    if (x_dpi < 72 || y_dpi < 72)
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_DPI;
        sjpegli_strcopy(result.error_message, "DPI values must be at least 72", JPEGLI_ERR_MSG_LEN);
        return result;
    }

    struct jpeg_compress_struct cinfo;
    struct JpegliErrorManager jerr;

    // track output buffer
    unsigned char *outbuffer = nullptr;
    unsigned long outsize = 0;

    // set up the error handler
    cinfo.err = jpegli_std_error(&jerr.pub);
    jerr.pub.error_exit = sjpegli_error_exit;

    if (setjmp(jerr.jump_buffer))
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = jerr.pub.msg_code;

        sjpegli_strcopy(result.error_message, jerr.last_error_msg, JPEGLI_ERR_MSG_LEN);

        // cleanup
        jpegli_destroy_compress(&cinfo);

        // free output buffer if it was allocated, only happens if we jumped here after starting
        // compression
        if (outbuffer)
        {
            std::free(outbuffer);
        }

        return result;
    }

    jpegli_create_compress(&cinfo);

    // allocate memory destination
    jpegli_mem_dest(&cinfo, &outbuffer, &outsize);

    cinfo.image_width = width;
    cinfo.image_height = height;
    cinfo.input_components = input_comps;
    cinfo.in_color_space = colorspace_t;

    if (config->xyb_mode)
    {
        jpegli_set_xyb_mode(&cinfo);
    }
    if (config->std_quant)
    {
        jpegli_use_standard_quant_tables(&cinfo);
    }

    jpegli_set_defaults(&cinfo);
    jpegli_set_quality(&cinfo, quality, TRUE);
    if (config->progressive)
    {
        jpegli_set_progressive_level(&cinfo, kDefaultProgressiveLevel);
    }
    else
    {
        jpegli_set_progressive_level(&cinfo, kDisableProgressive);
    }
    jpegli_enable_adaptive_quantization(&cinfo, config->adaptive_quantize);
    sjpegli_set_subsampling_factors(&cinfo, colorspace_t, subsampling, quality);
    cinfo.write_Adobe_marker =
        (colorspace_t == JCS_CMYK || colorspace_t == JCS_YCCK || colorspace_t == JCS_RGB) ? TRUE
                                                                                          : FALSE;

    cinfo.arith_code = FALSE; // disable arithmetic coding
    cinfo.write_JFIF_header = TRUE;
    cinfo.density_unit = 1; // dots per inch
    cinfo.X_density = static_cast<UINT16>(x_dpi);
    cinfo.Y_density = static_cast<UINT16>(y_dpi);
    cinfo.optimize_coding = config->optimize_coding ? TRUE : FALSE;

    jpegli_start_compress(&cinfo, TRUE);

    int row_stride = width * input_comps;

    while (cinfo.next_scanline < cinfo.image_height)
    {
        // const_cast is necessary because libjpeg legacy API expects non-const JSAMPROW
        JSAMPROW row_pointer[1] = {const_cast<JSAMPROW>(&pixels[cinfo.next_scanline * row_stride])};
        jpegli_write_scanlines(&cinfo, row_pointer, 1);
    }

    jpegli_finish_compress(&cinfo);
    jpegli_destroy_compress(&cinfo);

    result.success = SJPEGLI_SUCCESS;
    result.data = outbuffer;
    result.size = static_cast<size_t>(outsize);
    result.error_code = 0;

    return result;
}

void sjpegli_free_result(simple_jpegli_enc_result result)
{
    // this should be fine since the pointer would only be assigned
    // if all operations were successful, if not we cleaned up at the jump point
    if (result.data)
    {
        std::free(result.data);
    }
}

void sjpegli_get_version(simple_jpegli_version_t *out_version)
{
    if (out_version == nullptr)
    {
        return;
    }

    out_version->major = JPEGXL_MAJOR_VERSION;
    out_version->minor = JPEGXL_MINOR_VERSION;
    out_version->patch = JPEGXL_PATCH_VERSION;
    out_version->lib_ver = JPEG_LIB_VERSION;
}