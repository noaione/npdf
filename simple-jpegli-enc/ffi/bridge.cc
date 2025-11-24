#include "bridge.h"

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <csetjmp>

// Include the standard JPEG library (which links to Jpegli)
#include <jpeglib.h>

#define SJPEGLI_BAD_COLORSPACE 10 // Based on libjpeg error codes

// 1. Custom Error Manager
// We extend the standard error manager to include our jump buffer
struct JpegliErrorManager
{
    struct jpeg_error_mgr pub; // "Base class"
    jmp_buf jump_buffer;       // Context for longjmp
    char last_error_msg[JPEGLI_ERR_MSG_LEN];
};

// 2. The Error Callback
// This replaces the default "exit program" behavior of libjpeg
void sjpegli_error_exit(j_common_ptr cinfo)
{
    // Cast back to our custom struct
    auto *myerr = reinterpret_cast<JpegliErrorManager *>(cinfo->err);

    // Format the message provided by the library
    (*cinfo->err->format_message)(cinfo, myerr->last_error_msg);

    // Jump back to the setjmp point in the main function
    longjmp(myerr->jump_buffer, 1);
}

J_COLOR_SPACE sjpegli_convert_colorspace(simple_jpegli_colorspace_t colorspace) {
    switch (colorspace) {
        case GRAYSCALE: return JCS_GRAYSCALE;
        case RGB: return JCS_RGB;
        case YCbCr: return JCS_YCbCr;
        case CMYK: return JCS_CMYK;
        case YCCK: return JCS_YCCK;
        case EXT_RGB: return JCS_EXT_RGB;
        case EXT_RGBX: return JCS_EXT_RGBX;
        case EXT_BGR: return JCS_EXT_BGR;
        case EXT_BGRX: return JCS_EXT_BGRX;
        case EXT_XBGR: return JCS_EXT_XBGR;
        case EXT_XRGB: return JCS_EXT_XRGB;
        case EXT_RGBA: return JCS_EXT_RGBA;
        case EXT_BGRA: return JCS_EXT_BGRA;
        case EXT_ABGR: return JCS_EXT_ABGR;
        case EXT_ARGB: return JCS_EXT_ARGB;
        case RGB565: return JCS_RGB565;
        default: return JCS_UNKNOWN;
    }
}

int sjpegli_get_input_comps(J_COLOR_SPACE colorspace, int width) {
    switch (colorspace) {
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

// 3. The Main Wrapper
simple_jpegli_enc_result sjpegli_encode_pixels(const unsigned char *pixels, int width, int height, int quality, simple_jpegli_colorspace_t colorspace)
{
    // Initialize result with safe defaults
    simple_jpegli_enc_result result;
    result.data = nullptr;
    result.size = 0;
    result.success = 0;
    result.error_code = 0;
    std::memset(result.error_message, 0, JPEGLI_ERR_MSG_LEN);

    // Create colorspace
    J_COLOR_SPACE colorspace_t = sjpegli_convert_colorspace(colorspace);
    if (colorspace_t == JCS_UNKNOWN) {
        result.success = 0;
        result.error_code = SJPEGLI_BAD_COLORSPACE;
        std::strncpy(result.error_message, "Unsupported colorspace", JPEGLI_ERR_MSG_LEN - 1);
        return result;
    }

    int input_comps = sjpegli_get_input_comps(colorspace_t, width);
    if (input_comps == 0) {
        result.success = 0;
        result.error_code = SJPEGLI_BAD_COLORSPACE;
        std::strncpy(result.error_message, "Unsupported colorspace", JPEGLI_ERR_MSG_LEN - 1);
        return result;
    }

    struct jpeg_compress_struct cinfo;
    struct JpegliErrorManager jerr;

    // We must track the output buffer manually to free it if an error occurs
    unsigned char *outbuffer = nullptr;
    unsigned long outsize = 0;

    // Set up the error handler
    cinfo.err = jpeg_std_error(&jerr.pub);
    jerr.pub.error_exit = sjpegli_error_exit;

    if (setjmp(jerr.jump_buffer))
    {
        result.success = 0;
        result.error_code = jerr.pub.msg_code;

        // Copy the formatted message we captured in the callback
        std::strncpy(result.error_message, jerr.last_error_msg, JPEGLI_ERR_MSG_LEN - 1);

        // Cleanup libjpeg resources
        jpeg_destroy_compress(&cinfo);

        // If libjpeg allocated a buffer before crashing, free it
        if (outbuffer)
        {
            std::free(outbuffer);
        }

        return result;
    }

    // NORMAL EXECUTION BLOCK
    jpeg_create_compress(&cinfo);

    // Use the memory destination manager (standard in newer libjpeg/jpegli)
    // This tells libjpeg to allocate a buffer for us using malloc
    jpeg_mem_dest(&cinfo, &outbuffer, &outsize);

    cinfo.image_width = width;
    cinfo.image_height = height;
    cinfo.input_components = input_comps;
    cinfo.in_color_space = colorspace_t;

    jpeg_set_defaults(&cinfo);

    // This is where Jpegli applies its heuristics
    jpeg_set_quality(&cinfo, quality, TRUE);

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

    // Success! Populate result
    result.success = 1;
    result.data = outbuffer;
    result.size = static_cast<size_t>(outsize);
    result.error_code = 0;

    return result;
}

void sjpegli_free_result(simple_jpegli_enc_result result)
{
    if (result.data)
    {
        std::free(result.data);
    }
}
