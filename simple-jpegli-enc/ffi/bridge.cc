#include "bridge.h"

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <csetjmp>

#include <jpeglib.h>

#define SJPEGLI_BAD_COLORSPACE 10 // Based on libjpeg error codes
#define SJPEGLI_BAD_DPI 90 // Custom error code for bad DPI values

#define SJPEGLI_ERROR 0
#define SJPEGLI_SUCCESS 1

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

simple_jpegli_enc_result sjpegli_encode_pixels(
    const unsigned char *pixels,
    int width,
    int height,
    int quality,
    simple_jpegli_colorspace_t colorspace,
    unsigned int x_dpi,
    unsigned int y_dpi)
{
    // safe default to avoid UB
    simple_jpegli_enc_result result;
    result.data = nullptr;
    result.size = 0;
    result.success = 0;
    result.error_code = 0;
    std::memset(result.error_message, 0, JPEGLI_ERR_MSG_LEN);

    // convert to libjpeg colorspace
    J_COLOR_SPACE colorspace_t = sjpegli_convert_colorspace(colorspace);
    if (colorspace_t == JCS_UNKNOWN)
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_COLORSPACE;
        std::strncpy(result.error_message, "Unsupported colorspace", JPEGLI_ERR_MSG_LEN - 1);
        return result;
    }

    int input_comps = sjpegli_get_input_comps(colorspace_t, width);
    if (input_comps == 0)
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_COLORSPACE;
        std::strncpy(result.error_message, "Unsupported colorspace", JPEGLI_ERR_MSG_LEN - 1);
        return result;
    }

    // check if x_dpi and y_dpi is within uint16
    if (x_dpi < 0 || x_dpi > UINT16_MAX || y_dpi < 0 || y_dpi > UINT16_MAX)
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = SJPEGLI_BAD_DPI;
        std::strncpy(result.error_message, "DPI values must be between 0 and 65535", JPEGLI_ERR_MSG_LEN - 1);
        return result;
    }

    struct jpeg_compress_struct cinfo;
    struct JpegliErrorManager jerr;

    // track output buffer
    unsigned char *outbuffer = nullptr;
    unsigned long outsize = 0;

    // set up the error handler
    cinfo.err = jpeg_std_error(&jerr.pub);
    jerr.pub.error_exit = sjpegli_error_exit;

    if (setjmp(jerr.jump_buffer))
    {
        result.success = SJPEGLI_ERROR;
        result.error_code = jerr.pub.msg_code;

        std::strncpy(result.error_message, jerr.last_error_msg, JPEGLI_ERR_MSG_LEN - 1);

        // cleanup
        jpeg_destroy_compress(&cinfo);

        // free output buffer if it was allocated, only happens if we jumped here after starting compression
        if (outbuffer)
        {
            std::free(outbuffer);
        }

        return result;
    }

    jpeg_create_compress(&cinfo);

    // allocate memory destination
    jpeg_mem_dest(&cinfo, &outbuffer, &outsize);

    cinfo.image_width = width;
    cinfo.image_height = height;
    cinfo.input_components = input_comps;
    cinfo.in_color_space = colorspace_t;

    jpeg_set_defaults(&cinfo);
    jpeg_set_quality(&cinfo, quality, TRUE);
    jpeg_set_colorspace(&cinfo, colorspace_t);
    cinfo.write_Adobe_marker =
        (colorspace_t == JCS_CMYK ||
         colorspace_t == JCS_YCCK ||
         colorspace_t == JCS_RGB)
            ? TRUE
            : FALSE;

    cinfo.write_JFIF_header = TRUE;
    cinfo.density_unit = 1; // dots per inch
    cinfo.X_density = static_cast<UINT16>(x_dpi);
    cinfo.Y_density = static_cast<UINT16>(y_dpi);

    jpeg_start_compress(&cinfo, TRUE);

    int row_stride = width * input_comps;

    while (cinfo.next_scanline < cinfo.image_height)
    {
        // const_cast is necessary because libjpeg legacy API expects non-const JSAMPROW
        JSAMPROW row_pointer[1] = {
            const_cast<JSAMPROW>(&pixels[cinfo.next_scanline * row_stride])};
        jpeg_write_scanlines(&cinfo, row_pointer, 1);
    }

    jpeg_finish_compress(&cinfo);
    jpeg_destroy_compress(&cinfo);

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
