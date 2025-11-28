#ifndef SIMPLE_JPEGLI_ENC_BRIDGE_H
#define SIMPLE_JPEGLI_ENC_BRIDGE_H

#include <setjmp.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#include <jpeglib.h>

// A fixed-size buffer for error messages to avoid malloc complexity during crashes
#define JPEGLI_ERR_MSG_LEN 256

typedef enum simple_jpegli_colorspace {
    GRAYSCALE = 0,
    RGB = 1,
    YCbCr = 2,
    CMYK = 3,
    YCCK = 4,
    EXT_RGB = 5,
    EXT_RGBX = 6,
    EXT_BGR = 7,
    EXT_BGRX = 8,
    EXT_XBGR = 9,
    EXT_XRGB = 10,
    EXT_RGBA = 11,
    EXT_BGRA = 12,
    EXT_ABGR = 13,
    EXT_ARGB = 14,
    RGB565 = 15
} simple_jpegli_colorspace_t;

typedef enum simple_jpegli_subsampling {
    SUBSAMP_NONE = 0,
    SUBSAMP_AUTO = 1,
    SUBSAMP_S420 = 2,
    SUBSAMP_S422 = 3,
    SUBSAMP_S440 = 4,
    SUBSAMP_S444 = 5 // Same as no subsampling
} simple_jpegli_subsampling_t;

typedef struct {
    unsigned char* data;      // Pointer to the JPEG bytes
    size_t size;              // Size of the JPEG bytes
    int success;              // 1 = true, 0 = false
    int error_code;           // libjpeg error code
    char error_message[JPEGLI_ERR_MSG_LEN];
} simple_jpegli_enc_result;

typedef struct {
    int width;
    int height;
    int quality;
    unsigned int x_dpi;
    unsigned int y_dpi;
    simple_jpegli_colorspace_t colorspace;
    simple_jpegli_subsampling_t subsampling;
    bool progressive;
    bool adaptive_quantize;
    bool xyb_mode;
    bool std_quant;
    bool optimize_coding;
} simple_jpegli_enc_config;

simple_jpegli_enc_result sjpegli_encode_pixels(
    const unsigned char* pixels,
    const simple_jpegli_enc_config* config
);

// Frees the buffer allocated by libjpeg
void sjpegli_free_result(simple_jpegli_enc_result result);

typedef struct simple_jpegli_version {
    uint32_t major;
    uint32_t minor;
    uint32_t patch;
    uint32_t lib_ver;
} simple_jpegli_version_t;

void sjpegli_get_version(simple_jpegli_version_t *out_version);

#ifdef __cplusplus
}
#endif

#endif  // SIMPLE_JPEGLI_ENC_BRIDGE_H
