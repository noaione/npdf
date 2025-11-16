#ifndef TINY_POPPLER_SPLASH_BRIDGE_H
#define TINY_POPPLER_SPLASH_BRIDGE_H

#include <stddef.h>
#include <stdint.h>
#include <math.h>
#include <GfxState.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct splash_renderer splash_renderer_t;

typedef enum splash_color_mode {
    SPLASH_COLOR_MODE_MONO1 = 0,
    SPLASH_COLOR_MODE_MONO8 = 1,
    SPLASH_COLOR_MODE_RGB8 = 2,
    SPLASH_COLOR_MODE_BGR8 = 3,
    SPLASH_COLOR_MODE_XBGR8 = 4,
    SPLASH_COLOR_MODE_CMYK8 = 5,
    SPLASH_COLOR_MODE_DEVICEN8 = 6,
} splash_color_mode_t;

typedef enum splash_image_colorspace {
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
} splash_image_colorspace_t;

typedef enum splash_image_type {
    SPLASH_IMAGE_TYPE_UNKNOWN = 0,
    SPLASH_IMAGE_TYPE_IMAGE = 1,
    SPLASH_IMAGE_TYPE_STENCIL = 2,
    SPLASH_IMAGE_TYPE_MASK = 3,
    SPLASH_IMAGE_TYPE_SOFT_MASK = 4,
} splash_image_type_t;

typedef enum splash_crop_mode {
    SPLASH_CROP_MODE_MEDIA_BOX = 0,
    SPLASH_CROP_MODE_CROP_BOX = 1,
    // SPLASH_CROP_MODE_BLEED_BOX = 2,
    // SPLASH_CROP_MODE_TRIM_BOX = 3,
    // SPLASH_CROP_MODE_ART_BOX = 4,
} splash_crop_mode_t;

typedef struct splash_image {
    uint8_t *data;
    size_t len;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint32_t components;
    splash_color_mode_t color_mode;
    uint32_t bits_per_component;
} splash_image_t;

typedef struct splash_image_info {
    uint32_t width;
    uint32_t height;
    uint32_t components;
    uint32_t bits_per_component;
    int32_t xref_object;
    int32_t xref_generation;
    uint32_t page_number;
    double_t dpi_x;
    double_t dpi_y;
    // uint64_t total_objects;
    splash_image_type_t image_type;
    splash_image_colorspace_t colorspace;

    // Not exposed through FFI; only used internally inside the C++ wrapper
    const void *color_space_handle;
} splash_image_info_t;

typedef struct splash_page_info {
    uint32_t page_number;
    uint32_t image_count;
    uint64_t object_count;
} splash_page_info_t;

int splash_renderer_create(const char *path,
                           const char *owner_password,
                           const char *user_password,
                           splash_renderer_t **out_renderer,
                           char **error_out);
void splash_renderer_destroy(splash_renderer_t *renderer);

int splash_renderer_page_count(splash_renderer_t *renderer, uint32_t *out_count, char **error_out);

int splash_renderer_render_page(splash_renderer_t *renderer,
                                uint32_t page_index,
                                double dpi,
                                splash_color_mode_t color_mode,
                                splash_crop_mode_t crop_mode,
                                splash_image_t *out_image,
                                char **error_out);

int splash_renderer_collect_images(splash_renderer_t *renderer,
                                   splash_image_info_t **out_images,
                                   size_t *out_image_len,
                                   splash_page_info_t **out_pages,
                                   size_t *out_page_len,
                                   uint32_t page_start,
                                   uint32_t page_end,
                                   char **error_out);

void splash_renderer_free_image(splash_image_t *image);
void splash_renderer_free_cstr(char *message);
void splash_renderer_free_image_info(splash_image_info_t *images);
void splash_renderer_free_page_info(splash_page_info_t *pages);

//! Colorspace related functions
splash_image_colorspace_t gfxcs_get_color_mode(const void *cs_ptr);
typedef struct {
    uint32_t hival;
    const void *base; // opaque pointer to GfxColorSpace*
} colorspaces_indexed_info_t;

bool gfxcs_get_indexed_info(const void *cs_ptr, colorspaces_indexed_info_t *out);

typedef struct {
    const char *name;
    const void *alternate; // child colorspace
} colorspaces_separation_info_t;

bool gfxcs_get_separation_info(const void *cs_ptr, colorspaces_separation_info_t *out);

typedef struct {
    uint32_t count;
    const char **names;       // char*[count]
    const void *alternate;    // child colorspace
} colorspaces_devicen_info_t;

bool gfxcs_get_devicen_info(const void *cs_ptr, colorspaces_devicen_info_t *out);

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
} colorspaces_labxyz_info_t;

bool gfxcs_get_labxyz_info(const void *cs_ptr, colorspaces_labxyz_info_t *out);

typedef struct {
    const void *alternate; // alternative
} colorspaces_icc_info_t;

bool gfxcs_get_icc_info(const void *cs_ptr, colorspaces_icc_info_t *out);

void gfxcs_free_string(const char *s);
void gfxcs_free_string_array(const char **arr, uint32_t count);

#ifdef __cplusplus
}
#endif

#endif // TINY_POPPLER_SPLASH_BRIDGE_H
