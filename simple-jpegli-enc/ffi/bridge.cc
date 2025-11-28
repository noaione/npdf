#include "bridge.h"

#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <csetjmp>

#include <jpeglib.h>
#include <lib/jpegli/encode.h>

#define SJPEGLI_BAD_COLORSPACE 10 // Based on libjpeg error codes
#define SJPEGLI_BAD_INPUT 90      // Custom error code for bad input
#define SJPEGLI_BAD_DPI 91        // Custom error code for bad DPI values

#define SJPEGLI_ERROR 0
#define SJPEGLI_SUCCESS 1

namespace
{
    constexpr int kDisableProgressive = 0;
    constexpr int kDefaultProgressiveLevel = 2;

    struct JpegliErrorManager
    {
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
        case GRAYSCALE:
            return JCS_GRAYSCALE;
        case RGB:
            return JCS_RGB;
        case YCbCr:
            return JCS_YCbCr;
        case CMYK:
            return JCS_CMYK;
        case YCCK:
            return JCS_YCCK;
        case EXT_RGB:
            return JCS_EXT_RGB;
        case EXT_RGBX:
            return JCS_EXT_RGBX;
        case EXT_BGR:
            return JCS_EXT_BGR;
        case EXT_BGRX:
            return JCS_EXT_BGRX;
        case EXT_XBGR:
            return JCS_EXT_XBGR;
        case EXT_XRGB:
            return JCS_EXT_XRGB;
        case EXT_RGBA:
            return JCS_EXT_RGBA;
        case EXT_BGRA:
            return JCS_EXT_BGRA;
        case EXT_ABGR:
            return JCS_EXT_ABGR;
        case EXT_ARGB:
            return JCS_EXT_ARGB;
        case RGB565:
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

    void sjpegli_auto_subsampling_factors(
        j_compress_ptr cinfo,
        J_COLOR_SPACE colorspace,
        int quality)
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

        auto recompute_max_sampling = [&]()
        {
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

        auto set_luma_sampling = [&](int h_factor, int v_factor)
        {
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

    void sjpegli_set_subsampling_factors(
        j_compress_ptr cinfo,
        J_COLOR_SPACE colorspace,
        simple_jpegli_subsampling_t subsampling,
        int quality)
    {
        switch (subsampling)
        {
        case SUBSAMP_S420:
            cinfo->comp_info[0].h_samp_factor = 2;
            cinfo->comp_info[0].v_samp_factor = 2;
            return;
        case SUBSAMP_S422:
            cinfo->comp_info[0].h_samp_factor = 2;
            cinfo->comp_info[0].v_samp_factor = 1;
            return;
        case SUBSAMP_S440:
            cinfo->comp_info[0].h_samp_factor = 1;
            cinfo->comp_info[0].v_samp_factor = 2;
            return;
        case SUBSAMP_S444:
            cinfo->comp_info[0].h_samp_factor = 1;
            cinfo->comp_info[0].v_samp_factor = 1;
            return;
        case SUBSAMP_NONE:
            // Keep original sampling factors
            return;
        case SUBSAMP_AUTO:
        default:
            sjpegli_auto_subsampling_factors(cinfo, colorspace, quality);
            return;
        }
    }

    void debug_error(const char* msg)
    {
#ifdef SJPEGLI_DEBUG
        std::fprintf(stdout, "JPEGli debug: %s\n", msg);
        std::fflush(stdout);
#endif
    }
} // namespace

simple_jpegli_enc_result sjpegli_encode_pixels(const unsigned char *pixels, const simple_jpegli_enc_config *config)
{
    // safe default to avoid UB
    simple_jpegli_enc_result result;
    result.data = nullptr;
    result.size = 0;
    result.success = 0;
    result.error_code = 0;
    result.state = 0;
    std::memset(result.error_message, 0, JPEGLI_ERR_MSG_LEN);

    debug_error("Entered sjpegli_encode_pixels, checking inputs");
    if (pixels == nullptr || config == nullptr)
    {
        debug_error("Invalid input: nullptr detected");
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_INPUT;
        std::strncpy(result.error_message, "Invalid input, got nullptr", JPEGLI_ERR_MSG_LEN - 1);
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
        debug_error("Clamping quality to default 90");
        quality = 90; // reset to default quality
    }

    // convert to libjpeg colorspace
    debug_error("Converting colorspace");
    result.state = 1;
    J_COLOR_SPACE colorspace_t = sjpegli_convert_colorspace(colorspace);
    if (colorspace_t == JCS_UNKNOWN)
    {
        debug_error("Unsupported colorspace detected");
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_COLORSPACE;
        std::strncpy(result.error_message, "Unsupported colorspace", JPEGLI_ERR_MSG_LEN - 1);
        return result;
    }

    debug_error("Getting input components");
    result.state = 2;
    int input_comps = sjpegli_get_input_comps(colorspace_t, width);
    if (input_comps == 0)
    {
        debug_error("Unsupported colorspace detected");
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_COLORSPACE;
        std::strncpy(result.error_message, "Unsupported colorspace", JPEGLI_ERR_MSG_LEN - 1);
        return result;
    }

    // check if x_dpi and y_dpi is within uint16
    debug_error("Checking DPI values");
    result.state = 3;
    if (x_dpi > UINT16_MAX || y_dpi > UINT16_MAX)
    {
        debug_error("DPI values out of range");
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_DPI;
        std::strncpy(result.error_message, "DPI values must be between 0 and 65535", JPEGLI_ERR_MSG_LEN - 1);
        return result;
    }

    debug_error("Setting up JPEG compression structures");
    result.state = 4;
    struct jpeg_compress_struct cinfo;
    struct JpegliErrorManager jerr;

    // track output buffer
    unsigned char *outbuffer = nullptr;
    unsigned long outsize = 0;

    // set up the error handler
    debug_error("Setting up error handler with setjmp");
    result.state = 5;
    cinfo.err = jpegli_std_error(&jerr.pub);
    jerr.pub.error_exit = sjpegli_error_exit;

    debug_error("Setting up jump point for error handling");
    result.state = 6;
    if (setjmp(jerr.jump_buffer))
    {
        debug_error("Error occurred during JPEG compression");
        result.success = SJPEGLI_ERROR;
        result.error_code = jerr.pub.msg_code;

        std::strncpy(result.error_message, jerr.last_error_msg, JPEGLI_ERR_MSG_LEN - 1);

        // cleanup
        jpegli_destroy_compress(&cinfo);

        // free output buffer if it was allocated, only happens if we jumped here after starting compression
        if (outbuffer)
        {
            std::free(outbuffer);
        }

        return result;
    }

    debug_error("Creating JPEG compression object");
    result.state = 7;
    jpegli_create_compress(&cinfo);

    // allocate memory destination
    debug_error("Setting up memory destination for JPEG output");
    result.state = 8;
    jpegli_mem_dest(&cinfo, &outbuffer, &outsize);

    cinfo.image_width = width;
    cinfo.image_height = height;
    cinfo.input_components = input_comps;
    cinfo.in_color_space = colorspace_t;

    result.state = 8;
    if (config->xyb_mode)
    {
        debug_error("Enabling XYB mode");
        jpegli_set_xyb_mode(&cinfo);
    }
    result.state = 9;
    if (config->std_quant)
    {
        debug_error("Using standard quantization tables");
        jpegli_use_standard_quant_tables(&cinfo);
    }

    result.state = 10;
    debug_error("Setting compression parameters");
    jpegli_set_defaults(&cinfo);
    result.state = 11;
    jpegli_set_quality(&cinfo, quality, TRUE);
    result.state = 12;
    if (config->progressive)
    {
        debug_error("Enabling progressive encoding");
        jpegli_set_progressive_level(&cinfo, kDefaultProgressiveLevel);
    }
    else
    {
        debug_error("Disabling progressive encoding");
        jpegli_set_progressive_level(&cinfo, kDisableProgressive);
    }
    result.state = 13;
    jpegli_enable_adaptive_quantization(&cinfo, config->adaptive_quantize);
    result.state = 14;
    sjpegli_set_subsampling_factors(&cinfo, colorspace_t, subsampling, quality);
    result.state = 15;
    cinfo.write_Adobe_marker =
        (colorspace_t == JCS_CMYK ||
         colorspace_t == JCS_YCCK ||
         colorspace_t == JCS_RGB)
            ? TRUE
            : FALSE;

    cinfo.arith_code = FALSE; // disable arithmetic coding
    cinfo.write_JFIF_header = TRUE;
    cinfo.density_unit = 1; // dots per inch
    cinfo.X_density = static_cast<UINT16>(x_dpi);
    cinfo.Y_density = static_cast<UINT16>(y_dpi);
    cinfo.optimize_coding = config->optimize_coding ? TRUE : FALSE;

    debug_error("Starting JPEG compression");
    result.state = 16;
    jpegli_start_compress(&cinfo, TRUE);

    int row_stride = width * input_comps;

    debug_error("Writing scanlines");
    result.state = 17;
    int counter = 0;
    while (cinfo.next_scanline < cinfo.image_height)
    {
        // const_cast is necessary because libjpeg legacy API expects non-const JSAMPROW
        JSAMPROW row_pointer[1] = {
            const_cast<JSAMPROW>(&pixels[cinfo.next_scanline * row_stride])};
        jpegli_write_scanlines(&cinfo, row_pointer, 1);
        // log progress every rows
        char progress_msg[100];
        std::snprintf(progress_msg, sizeof(progress_msg), "Processed scanline %d", counter);
        debug_error(progress_msg);
        counter++;
    }

    debug_error("Finishing compression and cleaning up");
    result.state = 18;
    jpegli_finish_compress(&cinfo);
    debug_error("Destroying compression object");
    result.state = 19;
    jpegli_destroy_compress(&cinfo);

    debug_error("JPEG compression successful, preparing result");
    result.state = 20;
    result.success = SJPEGLI_SUCCESS;
    result.data = outbuffer;
    result.size = static_cast<size_t>(outsize);
    result.error_code = 0;

    debug_error("Exiting sjpegli_encode_pixels successfully");
    return result;
}

void sjpegli_free_result(simple_jpegli_enc_result result)
{
    // this should be fine since the pointer would only be assigned
    // if all operations were successful, if not we cleaned up at the jump point
    if (result.data)
    {
        debug_error("Freeing JPEG output buffer");
        std::free(result.data);
    }
}