#ifndef TINY_POPPLER_SPLASH_BRIDGE_H
#define TINY_POPPLER_SPLASH_BRIDGE_H

#include <GfxState.h>
#include <math.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ntsplash_renderer ntsplash_renderer_t;

typedef enum ntsplash_color_mode
{
    SPLASH_COLOR_MODE_MONO1 = 0,
    SPLASH_COLOR_MODE_MONO8 = 1,
    SPLASH_COLOR_MODE_RGB8 = 2,
    SPLASH_COLOR_MODE_BGR8 = 3,
    SPLASH_COLOR_MODE_XBGR8 = 4,
    SPLASH_COLOR_MODE_CMYK8 = 5,
    SPLASH_COLOR_MODE_DEVICEN8 = 6,
} ntsplash_color_mode_t;

typedef enum ntsplash_image_colorspace
{
    SPLASH_IMAGE_COLORSPACE_UNKNOWN = 0,
    SPLASH_IMAGE_COLORSPACE_DEVICE_GRAY = 1,
    SPLASH_IMAGE_COLORSPACE_DEVICE_RGB = 2,
    SPLASH_IMAGE_COLORSPACE_DEVICE_CMYK = 3,
    SPLASH_IMAGE_COLORSPACE_LAB = 4,
    SPLASH_IMAGE_COLORSPACE_ICC = 5,
    SPLASH_IMAGE_COLORSPACE_INDEXED = 6,
    SPLASH_IMAGE_COLORSPACE_PATTERN = 7,
    SPLASH_IMAGE_COLORSPACE_SEPARATION = 8,
    SPLASH_IMAGE_COLORSPACE_DEVICEN = 9,
} ntsplash_image_colorspace_t;

typedef enum ntsplash_image_type
{
    SPLASH_IMAGE_TYPE_UNKNOWN = 0,
    SPLASH_IMAGE_TYPE_IMAGE = 1,
    SPLASH_IMAGE_TYPE_STENCIL = 2,
    SPLASH_IMAGE_TYPE_MASK = 3,
    SPLASH_IMAGE_TYPE_SOFT_MASK = 4,
} ntsplash_image_type_t;

typedef enum ntsplash_crop_mode
{
    SPLASH_CROP_MODE_MEDIA_BOX = 0,
    SPLASH_CROP_MODE_CROP_BOX = 1,
    // SPLASH_CROP_MODE_BLEED_BOX = 2,
    // SPLASH_CROP_MODE_TRIM_BOX = 3,
    // SPLASH_CROP_MODE_ART_BOX = 4,
} ntsplash_crop_mode_t;

typedef enum ntsplash_zero_width_line_mode
{
    SPLASH_ZERO_WIDTH_LINE_DEFAULT = 0,  // use default behavior
    SPLASH_ZERO_WIDTH_LINE_HAIRLINE = 1, // draw zero-width lines as
    SPLASH_ZERO_WIDTH_LINE_NOTHING = 2   // do not draw zero-width lines
} ntsplash_zero_width_line_mode_t;

typedef struct ntsplash_image {
    uint8_t *data;
    size_t len;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint32_t components;
    ntsplash_color_mode_t color_mode;
    uint32_t bits_per_component;
} ntsplash_image_t;

typedef struct ntsplash_image_info {
    uint32_t width;
    uint32_t height;
    uint32_t components;
    uint32_t bits_per_component;
    int32_t xref_object;
    int32_t xref_generation;
    uint32_t page_number;
    double_t dpi_x;
    double_t dpi_y;

    ntsplash_image_type_t image_type;
    ntsplash_image_colorspace_t colorspace;

    // Matrix Info
    // [a, b, c, d, e, f] - The Affine Transform
    double ctm[6];

    // Not exposed through FFI; only used internally inside the C++ wrapper
    const void *color_space_handle;
} ntsplash_image_info_t;

typedef struct ntsplash_page_info {
    uint32_t page_number;
    uint32_t image_count;
    uint64_t object_count;

    uint8_t is_pdf_a_compatible;

    // Cropbox and mediabox in user space units
    // If all 0's, then not set
    double cropbox[4];
    double mediabox[4];
} ntsplash_page_info_t;

typedef struct ntsplash_version {
    uint32_t major;
    uint32_t minor;
    uint32_t patch;
} ntsplash_version_t;

int ntsplash_renderer_create(const char *path, const char *owner_password,
                             const char *user_password, ntsplash_renderer_t **out_renderer,
                             char **error_out);
void ntsplash_renderer_destroy(ntsplash_renderer_t *renderer);

int ntsplash_renderer_page_count(ntsplash_renderer_t *renderer, uint32_t *out_count,
                                 char **error_out);

int ntsplash_renderer_render_page(ntsplash_renderer_t *renderer, uint32_t page_index, double dpi,
                                  ntsplash_color_mode_t color_mode, ntsplash_crop_mode_t crop_mode,
                                  ntsplash_zero_width_line_mode_t zero_width_line_mode,
                                  ntsplash_image_t *out_image, char **error_out);

int ntsplash_renderer_collect_images(ntsplash_renderer_t *renderer,
                                     ntsplash_image_info_t **out_images, size_t *out_image_len,
                                     ntsplash_page_info_t **out_pages, size_t *out_page_len,
                                     uint32_t page_start, uint32_t page_end, char **error_out);

void ntsplash_renderer_free_image(ntsplash_image_t *image);
void ntsplash_renderer_free_cstr(char *message);
void ntsplash_renderer_free_image_info(ntsplash_image_info_t *images);
void ntsplash_renderer_free_page_info(ntsplash_page_info_t *pages);

void ntsplash_get_version(ntsplash_version_t *out_version);

//! Colorspace related functions
ntsplash_image_colorspace_t ntgfxcs_get_color_mode(const void *cs_ptr);
typedef struct {
    uint32_t hival;
    const void *base; // opaque pointer to GfxColorSpace*
} ntcolorspaces_indexed_info_t;

bool ntgfxcs_get_indexed_info(const void *cs_ptr, ntcolorspaces_indexed_info_t *out);

typedef struct {
    const char *name;
    const void *alternate; // child colorspace
} ntcolorspaces_separation_info_t;

bool ntgfxcs_get_separation_info(const void *cs_ptr, ntcolorspaces_separation_info_t *out);

typedef struct {
    uint32_t count;
    const char **names;    // char*[count]
    const void *alternate; // child colorspace
} ntcolorspaces_devicen_info_t;

bool ntgfxcs_get_devicen_info(const void *cs_ptr, ntcolorspaces_devicen_info_t *out);

typedef struct {
    double whiteX;
    double whiteY;
    double whiteZ;
    double blackX;
    double blackY;
    double blackZ;
    double minA;
    double minB;
    double maxA;
    double maxB;
} ntcolorspaces_labxyz_info_t;

bool ntgfxcs_get_labxyz_info(const void *cs_ptr, ntcolorspaces_labxyz_info_t *out);

typedef struct {
    const void *alternate; // alternative
} ntcolorspaces_icc_info_t;

bool ntgfxcs_get_icc_info(const void *cs_ptr, ntcolorspaces_icc_info_t *out);

void ntgfxcs_free_string(const char *s);
void ntgfxcs_free_string_array(const char **arr, uint32_t count);

#ifdef __cplusplus
}
#endif

#endif // TINY_POPPLER_SPLASH_BRIDGE_H
